//! Startup timing and memory checkpoints, logged under the `diag` target.

use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

/// Call first thing in `main`; later checkpoints measure from here.
pub fn start() {
    let _ = START.set(Instant::now());
}

pub fn checkpoint(phase: &str) {
    let elapsed = START
        .get()
        .map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0);
    match memory_stats::memory_stats() {
        Some(m) => log::info!(
            target: "diag",
            "{phase:<18} {elapsed:>8.1} ms  rss {:>7.1} MB  virt {:>7.1} MB",
            mb(m.physical_mem),
            mb(m.virtual_mem)
        ),
        None => log::info!(target: "diag", "{phase:<18} {elapsed:>8.1} ms"),
    }
}

fn mb(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}
