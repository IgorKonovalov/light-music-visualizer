//! macOS loopback capture via ScreenCaptureKit (macOS 13+).
//!
//! ScreenCaptureKit is the only first-party way to tap system audio output;
//! it requires the user to grant the *screen recording* permission (the
//! stream captures a 2x2 px, 1 fps throwaway video alongside the audio —
//! SCK will not run audio-only). The documented fallback is a virtual
//! device (BlackHole): set it as the output and this capture path is not
//! needed — that route lands with the capture-device work in the
//! live-performance plan.
//!
//! Same contract as `capture_win`: samples flow into the core's SPSC ring,
//! and the sample-handler callback does zero heap allocation, zero locks,
//! zero logging, zero file I/O (NFR section 5). SCK delivers audio on a
//! dedicated serial dispatch queue, which serializes access to the producer.

use std::cell::UnsafeCell;
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use lmv_core::audio::{AudioFormat, SampleConsumer, SampleProducer, intake};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_core_audio_types::AudioBufferList;
use objc2_core_foundation::CFRetained;
use objc2_core_media::{
    CMBlockBuffer, CMSampleBuffer, CMTime, CMTimeFlags,
    kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput,
    SCStreamOutputType,
};

/// Same headroom as the Windows path (~340 ms @ 48 kHz stereo).
const RING_CAPACITY_FRAMES: usize = 16_384;
/// The format we ask SCK to deliver; it resamples internally.
const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// Interleave scratch: whole frames only, preallocated once.
const SCRATCH_SAMPLES: usize = 32_768;
/// SCK audio arrives planar with one buffer per channel; cap what we read.
const MAX_PLANES: usize = 8;
/// Stack storage for the AudioBufferList header + up to MAX_PLANES buffers.
const ABL_STORAGE_BYTES: usize = 256;

#[derive(Debug)]
pub enum CaptureError {
    /// Shareable-content enumeration failed — usually the screen-recording
    /// permission was denied (System Settings > Privacy > Screen Recording).
    ShareableContent(String),
    NoDisplay,
    Stream(String),
    Format(lmv_core::audio::FormatError),
    Timeout,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::ShareableContent(e) => {
                write!(
                    f,
                    "ScreenCaptureKit content enumeration failed: {e} (screen-recording permission?)"
                )
            }
            CaptureError::NoDisplay => write!(f, "no display available to capture"),
            CaptureError::Stream(e) => write!(f, "ScreenCaptureKit stream error: {e}"),
            CaptureError::Format(e) => write!(f, "capture format rejected by core: {e}"),
            CaptureError::Timeout => write!(f, "timed out waiting for ScreenCaptureKit"),
        }
    }
}

impl std::error::Error for CaptureError {}

pub struct CaptureHandle {
    stream: Retained<SCStream>,
    // Kept alive for the stream's callbacks; released after stop.
    _output: Retained<StreamOutput>,
    _queue: DispatchRetained<DispatchQueue>,
    format: AudioFormat,
}

impl CaptureHandle {
    pub fn format(&self) -> AudioFormat {
        self.format
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        // Fire-and-forget stop; the retained output/queue outlive any
        // in-flight callback because we hold them until self drops.
        unsafe { self.stream.stopCaptureWithCompletionHandler(None) };
    }
}

/// Interleave planar channel data into `out`. Pure - unit tested below.
/// Returns the number of samples written (frames * planes).
fn interleave_planar(
    out: &mut [f32],
    planes: &[&[f32]],
    frame_offset: usize,
    frames: usize,
) -> usize {
    let channels = planes.len();
    let mut written = 0;
    for f in 0..frames {
        for plane in planes {
            out[written] = plane[frame_offset + f];
            written += 1;
        }
    }
    debug_assert_eq!(written, frames * channels);
    written
}

struct AudioState {
    producer: SampleProducer,
    scratch: Box<[f32]>,
}

/// Ivar wrapper. Safety: SCK invokes the output callback on the single
/// serial dispatch queue passed to addStreamOutput, so access is exclusive
/// without a lock (NFR section 5 forbids locking here anyway).
struct OutputIvars(UnsafeCell<AudioState>);
unsafe impl Send for OutputIvars {}
unsafe impl Sync for OutputIvars {}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AllocAnyThread]
    #[name = "LmvStreamOutput"]
    #[ivars = OutputIvars]
    struct StreamOutput;

    unsafe impl NSObjectProtocol for StreamOutput {}

    unsafe impl SCStreamOutput for StreamOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn stream_did_output_sample_buffer_of_type(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            output_type: SCStreamOutputType,
        ) {
            if output_type == SCStreamOutputType::Audio {
                // Safety: serial queue - see OutputIvars.
                unsafe { self.handle_audio(sample_buffer) };
            }
        }
    }
);

impl StreamOutput {
    fn new(producer: SampleProducer) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars(UnsafeCell::new(AudioState {
            producer,
            scratch: vec![0.0f32; SCRATCH_SAMPLES].into_boxed_slice(),
        })));
        unsafe { msg_send![super(this), init] }
    }

    /// Real-time path: stack ABL storage, no heap, no locks, no logging.
    unsafe fn handle_audio(&self, sample_buffer: &CMSampleBuffer) {
        let frames = unsafe { sample_buffer.num_samples() };
        if frames <= 0 {
            return;
        }
        let frames = frames as usize;

        #[repr(C, align(16))]
        struct AblStorage([u8; ABL_STORAGE_BYTES]);
        let mut storage = AblStorage([0; ABL_STORAGE_BYTES]);
        let abl_ptr = storage.0.as_mut_ptr().cast::<AudioBufferList>();
        let mut block_buffer: *mut CMBlockBuffer = std::ptr::null_mut();
        let status = unsafe {
            sample_buffer.audio_buffer_list_with_retained_block_buffer(
                std::ptr::null_mut(),
                abl_ptr,
                ABL_STORAGE_BYTES,
                None,
                None,
                kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
                &mut block_buffer,
            )
        };
        if status != 0 {
            return;
        }
        // Owns the block buffer; dropping releases the sample data reference.
        let _block_guard = NonNull::new(block_buffer).map(|p| unsafe { CFRetained::from_raw(p) });

        let abl = unsafe { &*abl_ptr };
        let plane_count = (abl.mNumberBuffers as usize).min(MAX_PLANES);
        if plane_count == 0 {
            return;
        }
        let buffers = unsafe { std::slice::from_raw_parts(abl.mBuffers.as_ptr(), plane_count) };

        let state = unsafe { &mut *self.ivars().0.get() };
        if plane_count == 1 {
            // Mono or already interleaved - push as-is.
            let buf = &buffers[0];
            if buf.mData.is_null() {
                return;
            }
            let samples = unsafe {
                std::slice::from_raw_parts(
                    buf.mData.cast::<f32>(),
                    buf.mDataByteSize as usize / std::mem::size_of::<f32>(),
                )
            };
            state.producer.push_samples(samples);
            return;
        }

        // Planar: interleave through the fixed scratch, chunked to its size.
        let mut planes: [&[f32]; MAX_PLANES] = [&[]; MAX_PLANES];
        for (slot, buf) in planes.iter_mut().zip(buffers.iter()) {
            if buf.mData.is_null() {
                return;
            }
            *slot = unsafe {
                std::slice::from_raw_parts(
                    buf.mData.cast::<f32>(),
                    buf.mDataByteSize as usize / std::mem::size_of::<f32>(),
                )
            };
        }
        let planes = &planes[..plane_count];
        let frames = frames.min(planes.iter().map(|p| p.len()).min().unwrap_or(0));
        let frames_per_chunk = state.scratch.len() / plane_count;
        let mut offset = 0;
        while offset < frames && frames_per_chunk > 0 {
            let n = (frames - offset).min(frames_per_chunk);
            let written = interleave_planar(&mut state.scratch, planes, offset, n);
            state.producer.push_samples(&state.scratch[..written]);
            offset += n;
        }
    }
}

/// Start system-audio capture. Blocks briefly while ScreenCaptureKit
/// enumerates content and starts the stream (first run triggers the
/// screen-recording permission prompt). Capture stops when the handle drops.
pub fn start() -> Result<(CaptureHandle, SampleConsumer), CaptureError> {
    let format = AudioFormat {
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
    };
    let (producer, consumer) =
        intake(format, RING_CAPACITY_FRAMES).map_err(CaptureError::Format)?;

    // 1. Enumerate shareable content (async -> block until the callback).
    let (content_tx, content_rx) = mpsc::channel();
    let content_block = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            let result = if content.is_null() {
                Err(describe_error(error))
            } else {
                // Safety: non-null content from the callback is valid.
                unsafe { Retained::retain(content) }.ok_or_else(|| "retain failed".to_string())
            };
            let _ = content_tx.send(result);
        },
    );
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&content_block) };
    let content = content_rx
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| CaptureError::Timeout)?
        .map_err(CaptureError::ShareableContent)?;

    // 2. Filter: the first display, excluding nothing - we only want audio.
    let displays = unsafe { content.displays() };
    let display = displays.firstObject().ok_or(CaptureError::NoDisplay)?;
    let filter = unsafe {
        SCContentFilter::initWithDisplay_excludingWindows(
            SCContentFilter::alloc(),
            &display,
            &NSArray::new(),
        )
    };

    // 3. Configuration: audio on, video minimized to a 2x2 px 1 fps stub.
    let config = unsafe { SCStreamConfiguration::new() };
    unsafe {
        config.setCapturesAudio(true);
        config.setExcludesCurrentProcessAudio(true);
        config.setSampleRate(SAMPLE_RATE as isize);
        config.setChannelCount(CHANNELS as isize);
        config.setWidth(2);
        config.setHeight(2);
        config.setMinimumFrameInterval(CMTime {
            value: 1,
            timescale: 1,
            flags: CMTimeFlags::Valid,
            epoch: 0,
        });
    }

    // 4. Stream + audio output on a dedicated serial queue.
    let output = StreamOutput::new(producer);
    let queue = DispatchQueue::new("lmv-sck-audio", None);
    let stream = unsafe {
        SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None)
    };
    unsafe {
        stream.addStreamOutput_type_sampleHandlerQueue_error(
            ProtocolObject::from_ref(&*output),
            SCStreamOutputType::Audio,
            Some(&queue),
        )
    }
    .map_err(|e| CaptureError::Stream(e.localizedDescription().to_string()))?;

    // 5. Start and wait for the completion callback.
    let (start_tx, start_rx) = mpsc::channel();
    let start_block = RcBlock::new(move |error: *mut NSError| {
        let result = if error.is_null() {
            Ok(())
        } else {
            Err(describe_error(error))
        };
        let _ = start_tx.send(result);
    });
    unsafe { stream.startCaptureWithCompletionHandler(Some(&start_block)) };
    start_rx
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| CaptureError::Timeout)?
        .map_err(CaptureError::Stream)?;

    Ok((
        CaptureHandle {
            stream,
            _output: output,
            _queue: queue,
            format,
        },
        consumer,
    ))
}

fn describe_error(error: *mut NSError) -> String {
    if error.is_null() {
        return "unknown error".to_string();
    }
    // Safety: non-null NSError from the callback is valid for the call.
    unsafe { (*error).localizedDescription() }.to_string()
}

#[cfg(test)]
mod tests {
    use super::interleave_planar;

    #[test]
    fn interleaves_planar_channels_in_frame_order() {
        let left = [1.0f32, 3.0, 5.0];
        let right = [2.0f32, 4.0, 6.0];
        let mut out = [0.0f32; 6];
        let written = interleave_planar(&mut out, &[&left, &right], 0, 3);
        assert_eq!(written, 6);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn interleaves_with_frame_offset() {
        let left = [0.0f32, 0.0, 7.0, 9.0];
        let right = [0.0f32, 0.0, 8.0, 10.0];
        let mut out = [0.0f32; 4];
        let written = interleave_planar(&mut out, &[&left, &right], 2, 2);
        assert_eq!(written, 4);
        assert_eq!(out, [7.0, 8.0, 9.0, 10.0]);
    }
}
