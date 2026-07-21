#[cfg(windows)]
mod capture_win;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

struct App {
    window: Option<Window>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let attrs = Window::default_attributes().with_title("light-music-visualizer");
            match event_loop.create_window(attrs) {
                Ok(window) => self.window = Some(window),
                Err(err) => {
                    eprintln!("failed to create window: {err}");
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let WindowEvent::CloseRequested = event {
            event_loop.exit();
        }
    }
}

fn main() {
    // Temporary Plan 0001 Phase 2 debug readout; replaced by the real render
    // wiring in Phase 4.
    if std::env::args().any(|a| a == "--meter") {
        run_meter();
        return;
    }

    // expect: init-time invariant — without an event loop there is no app.
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App { window: None };
    if let Err(err) = event_loop.run_app(&mut app) {
        eprintln!("event loop error: {err}");
        std::process::exit(1);
    }
}

/// Drains the capture ring on the main thread and prints a level readout —
/// the Phase 2 "live, non-glitching stream" check. The samples/interval
/// column should sit near sample_rate * channels * interval when audio plays.
#[cfg(windows)]
fn run_meter() {
    use std::time::{Duration, Instant};

    let (handle, mut consumer) = match capture_win::start() {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("loopback capture failed: {err}");
            std::process::exit(1);
        }
    };
    let format = handle.format();
    println!(
        "capturing default render device: {} Hz, {} ch — 15 s meter",
        format.sample_rate, format.channels
    );

    let mut buf = vec![0.0f32; 32_768];
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(250));
        let mut peak = 0.0f32;
        let mut sum_sq = 0.0f64;
        let mut total = 0usize;
        loop {
            let n = consumer.pop_samples(&mut buf);
            if n == 0 {
                break;
            }
            for &s in &buf[..n] {
                peak = peak.max(s.abs());
                sum_sq += f64::from(s) * f64::from(s);
            }
            total += n;
        }
        let rms = if total > 0 {
            (sum_sq / total as f64).sqrt()
        } else {
            0.0
        };
        let bar_len = ((rms.sqrt() * 40.0) as usize).min(40);
        println!(
            "samples {total:>6}  peak {peak:>6.3}  rms {rms:>6.4}  |{:<40}|",
            "#".repeat(bar_len)
        );
    }
    drop(handle);
    println!("meter done");
}

#[cfg(not(windows))]
fn run_meter() {
    eprintln!("--meter needs Windows loopback capture (mac path lands in Phase 9)");
    std::process::exit(1);
}
