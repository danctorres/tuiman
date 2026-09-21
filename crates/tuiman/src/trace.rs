//! `TUIMAN_TRACE=1`: startup phase timings and frame-time statistics, printed
//! to stderr on exit. The performance budgets in `docs/ARCHITECTURE.md` are
//! checked against this output.

use std::time::{Duration, Instant};

pub struct Trace {
    enabled: bool,
    start: Instant,
    marks: Vec<(&'static str, Duration)>,
    frames: u32,
    frame_total: Duration,
    frame_max: Duration,
}

impl Trace {
    pub fn start() -> Trace {
        Trace {
            enabled: std::env::var_os("TUIMAN_TRACE").is_some_and(|v| v != "0"),
            start: Instant::now(),
            marks: Vec::new(),
            frames: 0,
            frame_total: Duration::ZERO,
            frame_max: Duration::ZERO,
        }
    }

    /// Records the time since process start under `label`.
    pub fn mark(&mut self, label: &'static str) {
        if self.enabled {
            self.marks.push((label, self.start.elapsed()));
        }
    }

    pub fn frame(&mut self, took: Duration) {
        self.frames += 1;
        self.frame_total += took;
        self.frame_max = self.frame_max.max(took);
    }

    pub fn report(&self) {
        if !self.enabled {
            return;
        }
        eprintln!("tuiman trace");
        for (label, at) in &self.marks {
            eprintln!("  {label:<22} {:>8.2} ms", at.as_secs_f64() * 1e3);
        }
        let avg = self.frame_total.checked_div(self.frames).unwrap_or_default();
        eprintln!(
            "  frames {:<15} avg {:.2} ms, max {:.2} ms",
            self.frames,
            avg.as_secs_f64() * 1e3,
            self.frame_max.as_secs_f64() * 1e3
        );
    }
}
