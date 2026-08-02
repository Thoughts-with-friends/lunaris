//! Counting semaphore with a bounded wait and a reset operation.
//!
//! melonDS's local MP backend drives its FIFOs with
//! `Platform::Semaphore_{Create,Post,Reset,TryWait}`; the standard library
//! has no equivalent, so this is a minimal [`Condvar`]-based stand-in
//! providing exactly those three operations.
//!
//! The permit count lives in a named struct rather than being a bare
//! integer inside the mutex, because a `Mutex<u32>` trips
//! `clippy::mutex_integer` (denied workspace-wide).

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Permit count, guarded by [`Semaphore`]'s mutex.
#[derive(Debug, Default)]
struct Permits {
    count: u32,
}

/// A counting semaphore supporting post, reset, and timed wait.
#[derive(Debug, Default)]
pub struct Semaphore {
    permits: Mutex<Permits>,
    signal: Condvar,
}

impl Semaphore {
    /// Creates a semaphore holding no permits.
    #[must_use]
    pub fn new() -> Self {
        Semaphore::default()
    }

    /// Releases one permit, waking a single waiter.
    ///
    /// Saturates instead of overflowing; melonDS's platform semaphores are
    /// likewise never expected to accumulate `u32::MAX` un-consumed posts.
    pub fn post(&self) {
        let mut permits = self.lock();
        permits.count = permits.count.saturating_add(1);
        drop(permits);
        self.signal.notify_one();
    }

    /// Discards every outstanding permit.
    ///
    /// Used when a FIFO's read cursor is force-resynchronised to its write
    /// cursor, at which point the pending wake-ups no longer correspond to
    /// readable frames.
    pub fn reset(&self) {
        self.lock().count = 0;
    }

    /// Consumes one permit, waiting up to `timeout` for one to appear.
    ///
    /// Returns `true` if a permit was consumed. A zero `timeout` makes this
    /// a non-blocking poll, which is how melonDS expresses
    /// `RecvPacketGeneric(block = false)`.
    pub fn try_wait(&self, timeout: Duration) -> bool {
        let mut permits = self.lock();
        if permits.count > 0 {
            permits.count -= 1;
            return true;
        }
        if timeout.is_zero() {
            return false;
        }

        // `wait_timeout` may return spuriously, so loop against a deadline
        // rather than trusting a single wake-up.
        let deadline = Instant::now() + timeout;
        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            if remaining.is_zero() {
                return false;
            }
            let (next, _) = match self.signal.wait_timeout(permits, remaining) {
                Ok(pair) => pair,
                // A panicking holder cannot leave the count in a
                // structurally invalid state (it is a plain integer), so
                // recovering is preferable to poisoning the MP session.
                Err(poisoned) => poisoned.into_inner(),
            };
            permits = next;
            if permits.count > 0 {
                permits.count -= 1;
                return true;
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Permits> {
        self.permits.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_then_try_wait_succeeds_without_blocking() {
        let sem = Semaphore::new();
        sem.post();
        assert!(sem.try_wait(Duration::ZERO));
        assert!(!sem.try_wait(Duration::ZERO));
    }

    #[test]
    fn reset_discards_pending_permits() {
        let sem = Semaphore::new();
        sem.post();
        sem.post();
        sem.reset();
        assert!(!sem.try_wait(Duration::ZERO));
    }

    #[test]
    fn try_wait_times_out_when_no_permit_arrives() {
        let sem = Semaphore::new();
        let start = Instant::now();
        assert!(!sem.try_wait(Duration::from_millis(20)));
        assert!(start.elapsed() >= Duration::from_millis(15));
    }

    #[test]
    fn try_wait_wakes_on_a_post_from_another_thread() {
        let sem = std::sync::Arc::new(Semaphore::new());
        let poster = std::sync::Arc::clone(&sem);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            poster.post();
        });
        assert!(sem.try_wait(Duration::from_secs(2)));
    }
}
