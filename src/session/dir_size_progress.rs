//! Throttled partial totals for directory-size walks.

use std::time::{Duration, Instant};

use flume::Sender;

pub struct Throttle {
    steps: u32,
    last_bytes: u64,
    since: Instant,
}

impl Throttle {
    pub fn new() -> Self {
        Self {
            steps: 0,
            last_bytes: 0,
            since: Instant::now(),
        }
    }

    pub fn maybe_send(&mut self, total: u64, progress: &Sender<u64>) {
        self.steps = self.steps.wrapping_add(1);
        if self.steps == 1
            || self.steps % 64 == 0
            || total.saturating_sub(self.last_bytes) >= 1_048_576
            || self.since.elapsed() >= Duration::from_millis(250)
        {
            self.last_bytes = total;
            self.since = Instant::now();
            let _ = progress.send(total);
        }
    }

    pub fn flush(&mut self, total: u64, progress: &Sender<u64>) {
        if total != self.last_bytes {
            self.last_bytes = total;
            let _ = progress.send(total);
        }
    }
}
