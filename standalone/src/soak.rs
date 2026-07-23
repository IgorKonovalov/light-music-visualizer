//! Long-run soak instrumentation for the standalone (Plan 0009 Phase 5).
//!
//! With `--soak <path>`, appends one sample every few seconds — elapsed time,
//! fps, resident-set bytes, total frames, and a heartbeat counter — so a
//! multi-hour session yields a measurable fps/RSS trace to check for drift or
//! stalls (Phase 6's ≥4-hour run reads it). Off by default: when `--soak` isn't
//! passed there is no `SoakLog` at all, so the render loop is byte-unchanged.
//!
//! Lives on the render/UI thread only (never the sacred audio callback). The
//! per-frame cost is a single elapsed-time comparison that returns immediately;
//! the actual file write happens only on the coarse sample tick, off the
//! per-frame hot path. RSS is reused from the diagnostics query (`rss.rs`) — no
//! new dependency, no second OS binding.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lmv_core::diag::Metrics;

/// One soak sample every few seconds — coarse enough to stay off the per-frame
/// path, fine enough that a multi-hour run has thousands of points.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);
/// Column header written when a fresh soak log is created.
const HEADER: &str = "elapsed_secs\tfps\trss_bytes\tframes_total\theartbeat\n";

/// Appends periodic soak samples to a log file. Constructed only when `--soak`
/// is requested, so its mere existence signals the mode is on.
pub struct SoakLog {
    path: PathBuf,
    file: Option<File>,
    /// Session start, for the elapsed-time column.
    start: Instant,
    /// Deadline pacing: the wall-clock time of the last written sample.
    last: Instant,
    /// Monotonic sample counter — a heartbeat that must keep climbing for the
    /// whole run (a frozen heartbeat in the log means the render thread stalled).
    heartbeat: u64,
}

impl SoakLog {
    /// Start a soak logger writing to `path`. The file is opened lazily on the
    /// first sample.
    pub fn new(path: PathBuf) -> Self {
        // Soak timing reads the wall clock in the shell; core analysis stays
        // clock-free (determinism).
        #[allow(
            clippy::disallowed_methods,
            reason = "soak-cadence start on the render thread; core analysis stays clock-free"
        )]
        let now = Instant::now();
        eprintln!("soak mode: logging to {}", path.display());
        Self {
            path,
            file: None,
            start: now,
            last: now,
            heartbeat: 0,
        }
    }

    /// Write a sample if the interval has elapsed. Returns immediately otherwise,
    /// so the per-frame cost is just the elapsed check. `rss` is evaluated lazily
    /// — only when a sample is actually due — to avoid a per-frame OS query.
    #[allow(
        clippy::disallowed_methods,
        reason = "soak-cadence pacing on the render thread; core analysis stays clock-free"
    )]
    pub fn maybe_sample(&mut self, metrics: &Metrics, rss: impl FnOnce() -> Option<u64>) {
        if self.last.elapsed() < SAMPLE_INTERVAL {
            return;
        }
        self.last = Instant::now();

        if self.file.is_none() {
            self.open();
        }
        let elapsed = self.start.elapsed().as_secs_f64();
        let Some(file) = self.file.as_mut() else {
            return;
        };

        self.heartbeat += 1;
        let rss = rss().unwrap_or(0);
        let _ = writeln!(
            file,
            "{elapsed:.1}\t{:.1}\t{rss}\t{}\t{}",
            metrics.fps, metrics.frames_total, self.heartbeat,
        );
        let _ = file.flush();
    }

    /// Open (creating dirs and the file) for appending, writing the header if
    /// the file is new. A failure is reported once and leaves the log dormant.
    fn open(&mut self) {
        let path: &Path = &self.path;
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
            Err(err) => eprintln!("soak log unavailable ({err})"),
        }
    }
}
