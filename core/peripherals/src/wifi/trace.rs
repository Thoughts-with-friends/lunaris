//! Association / MP frame trace, shared by every console in the process.
//!
//! Answers one question: when an association response reaches the RX ring
//! but the driver never writes `W_AID_LOW`, does the driver read back the
//! bytes that were written? Lines are printed to stderr and kept (the last
//! [`LIMIT`]) for front ends without a console.
//!
//! Enable with `LUNARIS_MP_ASSOC_TRACE=1` or [`set`] at run time.
use std::sync::{
    Mutex, Once,
    atomic::{AtomicBool, Ordering},
};

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Seeds [`ENABLED`] from the environment once.
static INIT: Once = Once::new();
static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// How many lines are retained (the trace is armed around a reproduction).
pub const LIMIT: usize = 500;

/// Whether the trace is on.
pub fn enabled() -> bool {
    INIT.call_once(|| {
        if std::env::var_os("LUNARIS_MP_ASSOC_TRACE").is_some() {
            ENABLED.store(true, Ordering::Relaxed);
        }
    });
    ENABLED.load(Ordering::Relaxed)
}

/// Turns the trace on or off.
pub fn set(on: bool) {
    // Seed first, so a later lazy seed cannot undo this.
    let _ = enabled();
    ENABLED.store(on, Ordering::Relaxed);
}

/// Records one line (and echoes it to stderr).
pub fn log(line: String) {
    eprintln!("{line}");
    let mut lines = LINES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if lines.len() >= LIMIT {
        lines.remove(0);
    }
    lines.push(line);
}

/// The retained lines, oldest first.
pub fn lines() -> Vec<String> {
    LINES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Discards the retained lines.
pub fn clear() {
    LINES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
}
