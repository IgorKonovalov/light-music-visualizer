//! WASAPI loopback capture: taps whatever the default render device is
//! playing and feeds it to the core's sample intake.
//!
//! WASAPI loopback does not deliver reliable event-callback wakeups, so a
//! dedicated thread polls the capture client every few milliseconds. The
//! polling loop is this app's "audio callback" and obeys NFR section 5:
//! after stream start it performs zero heap allocation, zero locks, zero
//! logging, zero file I/O — it copies device buffers into the SPSC ring and
//! sleeps.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use lmv_core::audio::{AudioFormat, SampleConsumer, SampleProducer, intake};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
    IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE, eConsole, eRender,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize,
};

/// Ring headroom: ~340 ms at 48 kHz. Capacity is deliberately larger than the
/// latency budget — NFR section 3 requires reading near the write head, which
/// the consumer does by draining every frame, not by keeping the ring small.
const RING_CAPACITY_FRAMES: usize = 16_384;

/// WASAPI shared-mode periods are 10 ms; polling faster than the period keeps
/// delivery latency well under the 15 ms capture allocation in NFR section 3.
const POLL_INTERVAL: Duration = Duration::from_millis(4);

/// Requested WASAPI buffer duration (100 ms, in 100 ns units) — device-side
/// headroom so a late poll drops nothing.
const BUFFER_DURATION_HNS: i64 = 1_000_000;

/// Scratch zeros pushed when a packet carries the SILENT flag (its data
/// pointer is not required to be valid then). Preallocated before the loop.
const SILENCE_CHUNK_SAMPLES: usize = 4096;

pub struct CaptureHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    format: AudioFormat,
}

impl CaptureHandle {
    pub fn format(&self) -> AudioFormat {
        self.format
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Debug)]
pub enum CaptureError {
    Windows(windows::core::Error),
    UnsupportedMixFormat(String),
    Format(lmv_core::audio::FormatError),
    ThreadDied,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::Windows(e) => write!(f, "WASAPI error: {e}"),
            CaptureError::UnsupportedMixFormat(what) => {
                write!(f, "unsupported mix format: {what}")
            }
            CaptureError::Format(e) => write!(f, "mix format rejected by core: {e}"),
            CaptureError::ThreadDied => write!(f, "capture thread died during setup"),
        }
    }
}

impl std::error::Error for CaptureError {}

impl From<windows::core::Error> for CaptureError {
    fn from(e: windows::core::Error) -> Self {
        CaptureError::Windows(e)
    }
}

/// Start loopback capture of the default render device. Blocks until the
/// stream is running, then returns the handle plus the consumer half of the
/// ring. Capture stops when the handle is dropped.
pub fn start() -> Result<(CaptureHandle, SampleConsumer), CaptureError> {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    // One-shot: the capture thread reports its setup result (and the ring
    // consumer) back before entering the real-time loop.
    let (setup_tx, setup_rx) = mpsc::channel();

    let thread = std::thread::Builder::new()
        .name("wasapi-loopback".into())
        .spawn(move || capture_thread(&thread_stop, &setup_tx))
        .expect("spawning the capture thread is an init-time invariant");

    match setup_rx.recv() {
        Ok(Ok((format, consumer))) => Ok((
            CaptureHandle {
                stop,
                thread: Some(thread),
                format,
            },
            consumer,
        )),
        Ok(Err(e)) => {
            stop.store(true, Ordering::Release);
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            stop.store(true, Ordering::Release);
            let _ = thread.join();
            Err(CaptureError::ThreadDied)
        }
    }
}

type SetupResult = Result<(AudioFormat, SampleConsumer), CaptureError>;

struct Stream {
    audio_client: IAudioClient,
    capture_client: IAudioCaptureClient,
    producer: SampleProducer,
    // Interleaving width of the captured stream. The ring producer no longer
    // exposes the format (Plan 0005), so carry the channel count here.
    channels: usize,
}

fn capture_thread(stop: &AtomicBool, setup_tx: &mpsc::Sender<SetupResult>) {
    // COM init and all WASAPI calls stay on this one thread.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if let Err(e) = com.ok() {
        let _ = setup_tx.send(Err(e.into()));
        return;
    }

    match setup_stream() {
        Ok((mut stream, format, consumer)) => {
            let _ = setup_tx.send(Ok((format, consumer)));
            run_capture_loop(&mut stream, stop);
            unsafe {
                let _ = stream.audio_client.Stop();
            }
        }
        Err(e) => {
            let _ = setup_tx.send(Err(e));
        }
    }

    unsafe { CoUninitialize() };
}

fn setup_stream() -> Result<(Stream, AudioFormat, SampleConsumer), CaptureError> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let audio_client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

        let mix_format = audio_client.GetMixFormat()?;
        let parsed = parse_mix_format(mix_format);
        let init_result = parsed.as_ref().ok().map(|_| {
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                BUFFER_DURATION_HNS,
                0,
                mix_format,
                None,
            )
        });
        CoTaskMemFree(Some(mix_format as *const _));
        let format = parsed?;
        if let Some(r) = init_result {
            r?;
        }

        let core_format = AudioFormat {
            sample_rate: format.sample_rate,
            channels: format.channels,
        }
        .validate()
        .map_err(CaptureError::Format)?;

        let capture_client: IAudioCaptureClient = audio_client.GetService()?;
        audio_client.Start()?;

        let (producer, consumer) =
            intake(core_format, RING_CAPACITY_FRAMES).map_err(CaptureError::Format)?;
        Ok((
            Stream {
                audio_client,
                capture_client,
                producer,
                channels: core_format.channels as usize,
            },
            core_format,
            consumer,
        ))
    }
}

/// The real-time loop. From here until `stop` flips: no allocation, no locks,
/// no logging, no I/O — copy packets into the ring, release, sleep.
fn run_capture_loop(stream: &mut Stream, stop: &AtomicBool) {
    // Preallocated so silent packets cost no heap work inside the loop.
    let silence = [0.0f32; SILENCE_CHUNK_SAMPLES];
    let channels = stream.channels;
    let Stream {
        capture_client,
        producer,
        ..
    } = stream;

    while !stop.load(Ordering::Acquire) {
        loop {
            let packet_frames = match unsafe { capture_client.GetNextPacketSize() } {
                Ok(n) => n,
                Err(_) => break,
            };
            if packet_frames == 0 {
                break;
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames_read: u32 = 0;
            let mut flags: u32 = 0;
            let got = unsafe {
                capture_client.GetBuffer(&mut data, &mut frames_read, &mut flags, None, None)
            };
            if got.is_err() {
                break;
            }
            let sample_count = frames_read as usize * channels;
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                push_silence(producer, &silence, sample_count, channels);
            } else if !data.is_null() && sample_count > 0 {
                // Safety: WASAPI hands us frames_read frames of the mix
                // format we validated as f32 at setup.
                let samples =
                    unsafe { std::slice::from_raw_parts(data as *const f32, sample_count) };
                // Drop-on-full is the ring's policy; nothing to retry
                // without blocking.
                let _ = producer.push_samples(samples);
            }
            if unsafe { capture_client.ReleaseBuffer(frames_read) }.is_err() {
                break;
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn push_silence(
    producer: &mut SampleProducer,
    silence: &[f32],
    mut remaining: usize,
    channels: usize,
) {
    let chunk_max = silence.len() / channels * channels;
    while remaining > 0 && chunk_max > 0 {
        let n = remaining.min(chunk_max);
        let written = producer.push_samples(&silence[..n]);
        if written < n {
            break;
        }
        remaining -= n;
    }
}

struct MixFormat {
    sample_rate: u32,
    channels: u16,
}

/// Shared-mode mix format is float32 PCM on every modern Windows box; anything
/// else is rejected here at the boundary rather than guessed at.
unsafe fn parse_mix_format(fmt: *const WAVEFORMATEX) -> Result<MixFormat, CaptureError> {
    // WAVEFORMATEX is declared packed — copy it out unaligned before reading
    // fields; taking references into it is UB.
    let base = unsafe { std::ptr::read_unaligned(fmt) };
    // Field reads below copy out of the (still packed) local — never take
    // references into it, which is what E0793 forbids.
    let tag = base.wFormatTag;
    let bits = base.wBitsPerSample;
    let sample_rate = base.nSamplesPerSec;
    let channels = base.nChannels;
    let is_float = if tag as u32 == WAVE_FORMAT_IEEE_FLOAT {
        true
    } else if tag as u32 == WAVE_FORMAT_EXTENSIBLE {
        let ext = unsafe { std::ptr::read_unaligned(fmt as *const WAVEFORMATEXTENSIBLE) };
        let sub = ext.SubFormat;
        sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
    } else {
        false
    };
    if !is_float || bits != 32 {
        return Err(CaptureError::UnsupportedMixFormat(format!(
            "tag={tag} bits={bits}"
        )));
    }
    Ok(MixFormat {
        sample_rate,
        channels,
    })
}
