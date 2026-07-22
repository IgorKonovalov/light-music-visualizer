//! ~1 Hz structured diagnostics logger for the standalone (Plan 0011).
//!
//! Appends one tab-separated sample per second to a rotating `diagnostics.log`
//! under the per-user app dir. Lives on the render/UI thread only — never the
//! capture/audio thread (the audio callback is sacred). File I/O at 1 Hz on the
//! render thread is well within budget.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lmv_core::diag::Metrics;

/// One sample per second.
const LOG_INTERVAL: Duration = Duration::from_secs(1);
/// Rotate the active log once it passes this size (keeps one `.1` backup).
const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// Column header written when a fresh log is created.
const HEADER: &str = "unix_ms\tfps\tframe_ms_avg\tframe_ms_p99\tframes_total\tframes_dropped\tgpu_bytes\trss_bytes\n";

/// Appends diagnostics samples to a rotating log file at ~1 Hz.
pub struct DiagLog {
    path: Option<PathBuf>,
    file: Option<File>,
    last: Instant,
}

impl DiagLog {
    /// A logger writing to `path` (an unresolved `None` path silently no-ops, so
    /// a machine without a resolvable data dir still runs — degrade, never crash).
    pub fn new(path: Option<PathBuf>) -> Self {
        // Log-cadence pacing reads the wall clock in the shell; core analysis
        // stays clock-free (determinism).
        #[allow(
            clippy::disallowed_methods,
            reason = "log-cadence start on the render thread; core analysis stays clock-free"
        )]
        let last = Instant::now();
        Self {
            path,
            file: None,
            last,
        }
    }

    /// Write a sample if a second has elapsed since the last. `rss` is evaluated
    /// lazily — only when a sample is actually due — to avoid a per-frame OS query.
    #[allow(
        clippy::disallowed_methods,
        reason = "log-cadence + sample timestamp reads on the render thread; core analysis stays clock-free"
    )]
    pub fn maybe_log(&mut self, metrics: &Metrics, rss: impl FnOnce() -> Option<u64>) {
        if self.last.elapsed() < LOG_INTERVAL {
            return;
        }
        self.last = Instant::now();

        let Some(path) = self.path.clone() else {
            return;
        };
        self.rotate_if_large(&path);
        if self.file.is_none() {
            self.open(&path);
        }
        let Some(file) = self.file.as_mut() else {
            return;
        };

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let rss = rss().unwrap_or(0);
        let _ = writeln!(
            file,
            "{ts}\t{:.1}\t{:.3}\t{:.3}\t{}\t{}\t{}\t{}",
            metrics.fps,
            metrics.frame_ms_avg,
            metrics.frame_ms_p99,
            metrics.frames_total,
            metrics.frames_dropped,
            metrics.gpu_bytes,
            rss,
        );
        let _ = file.flush();
    }

    /// Open (creating dirs and the file) for appending, writing the header if the
    /// file is new. A failure is reported once and leaves the logger dormant.
    fn open(&mut self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let is_new = !path.exists();
        match OpenOptions::new().create(true).append(true).open(path) {
            Ok(mut file) => {
                if is_new {
                    let _ = file.write_all(HEADER.as_bytes());
                }
                self.file = Some(file);
            }
            Err(err) => eprintln!("diagnostics log unavailable ({err})"),
        }
    }

    /// Rotate to a single `.1` backup once the active log grows past the cap.
    fn rotate_if_large(&mut self, path: &Path) {
        let too_big = fs::metadata(path)
            .map(|m| m.len() > MAX_LOG_BYTES)
            .unwrap_or(false);
        if too_big {
            self.file = None; // close before renaming
            let backup = path.with_extension("log.1");
            let _ = fs::rename(path, &backup);
        }
    }
}
