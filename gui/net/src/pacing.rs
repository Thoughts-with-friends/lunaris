//! Adaptive link-speed controller: turns measured reply-success rate,
//! jitter, and per-frame block time into the run-ahead window and receive
//! timeout consumed by [`crate::transport::NetTransport`]. See
//! `docs/design/design_lan.md` §9.
//!
//! Pure and dependency-free so it's testable without any socket involved:
//! [`Controller::evaluate`] takes a snapshot of measurements and returns
//! the (possibly unchanged) parameters for the next evaluation window.

use std::time::Duration;

use nds_core::nds::LinkHints;

pub const RUNAHEAD_MIN_US: u32 = 250;
pub const RUNAHEAD_MAX_US: u32 = 16_000;
pub const RECV_TIMEOUT_MIN_MS: u16 = 2;
pub const RECV_TIMEOUT_MAX_MS: u16 = 40;

/// One evaluation window's worth of measurements feeding the controller.
/// See `docs/design/design_lan.md` §9.1.
#[derive(Debug, Clone, Copy, Default)]
pub struct Measurements {
    /// Fraction of requested replies actually collected, in `0.0..=1.0`.
    pub reply_success: f32,
    /// EWMA inter-arrival jitter, in microseconds.
    pub jitter_us: u32,
    /// Average wall time `recv_host_packet` blocked per emulated frame.
    pub blocked_ms_avg: u16,
}

/// Adaptive controller state. Owned by the host; evaluated roughly once
/// per second (on each `Heartbeat` round), per
/// `docs/design/design_lan.md` §9.3.
#[derive(Debug, Clone, Copy)]
pub struct Controller {
    runahead_us: u32,
    recv_timeout_ms: u16,
    stable_evaluations: u32,
    auto: bool,
}

impl Default for Controller {
    fn default() -> Self {
        Controller { runahead_us: 1000, recv_timeout_ms: 8, stable_evaluations: 0, auto: true }
    }
}

impl Controller {
    pub const fn new() -> Self {
        Controller { runahead_us: 1000, recv_timeout_ms: 8, stable_evaluations: 0, auto: true }
    }

    /// Switches between automatic adjustment and a user-pinned fixed value.
    /// In fixed mode [`Controller::evaluate`] still tracks `stable_evaluations`
    /// bookkeeping but never changes `runahead_us`/`recv_timeout_ms`.
    pub fn set_auto(&mut self, auto: bool) {
        self.auto = auto;
    }

    pub fn set_fixed(&mut self, runahead_us: u32, recv_timeout_ms: u16) {
        self.runahead_us = runahead_us.clamp(RUNAHEAD_MIN_US, RUNAHEAD_MAX_US);
        self.recv_timeout_ms = recv_timeout_ms.clamp(RECV_TIMEOUT_MIN_MS, RECV_TIMEOUT_MAX_MS);
    }

    pub const fn hints(&self) -> LinkHints {
        LinkHints {
            runahead_us: self.runahead_us,
            recv_timeout: Duration::from_millis(self.recv_timeout_ms as u64),
        }
    }

    pub const fn runahead_us(&self) -> u32 {
        self.runahead_us
    }

    pub const fn recv_timeout_ms(&self) -> u16 {
        self.recv_timeout_ms
    }

    /// Applies one evaluation window's measurements, following the rule
    /// order in `docs/design/design_lan.md` §9.3.
    pub fn evaluate(&mut self, m: Measurements) {
        if !self.auto {
            return;
        }

        if m.reply_success < 0.90 {
            self.runahead_us = (self.runahead_us.saturating_mul(2)).min(RUNAHEAD_MAX_US);
            self.recv_timeout_ms = (self.recv_timeout_ms + 4).min(RECV_TIMEOUT_MAX_MS);
            self.stable_evaluations = 0;
        } else if m.reply_success < 0.98 {
            self.runahead_us = (self.runahead_us.saturating_mul(3) / 2).min(RUNAHEAD_MAX_US);
            self.stable_evaluations = 0;
        } else if m.reply_success > 0.995 {
            self.stable_evaluations += 1;
            if self.stable_evaluations >= 10 {
                self.runahead_us = (self.runahead_us * 4 / 5).max(RUNAHEAD_MIN_US);
                self.recv_timeout_ms =
                    self.recv_timeout_ms.saturating_sub(1).max(RECV_TIMEOUT_MIN_MS);
            }
        }

        if m.jitter_us > self.runahead_us {
            self.runahead_us = (m.jitter_us.saturating_mul(2)).min(RUNAHEAD_MAX_US);
            self.stable_evaluations = 0;
        }

        if m.blocked_ms_avg > 24 {
            self.runahead_us = (self.runahead_us.saturating_mul(2)).min(RUNAHEAD_MAX_US);
            self.recv_timeout_ms = (self.recv_timeout_ms + 4).min(RECV_TIMEOUT_MAX_MS);
            self.stable_evaluations = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poor_reply_success_raises_runahead() {
        let mut c = Controller::new();
        let before = c.runahead_us();
        c.evaluate(Measurements { reply_success: 0.5, jitter_us: 0, blocked_ms_avg: 0 });
        assert!(c.runahead_us() > before);
    }

    #[test]
    fn sustained_good_link_lowers_runahead() {
        let mut c = Controller::new();
        c.set_fixed(4000, 20);
        for _ in 0..15 {
            c.evaluate(Measurements { reply_success: 0.999, jitter_us: 0, blocked_ms_avg: 0 });
        }
        assert!(c.runahead_us() < 4000);
    }

    #[test]
    fn fixed_mode_ignores_measurements() {
        let mut c = Controller::new();
        c.set_fixed(2000, 10);
        c.set_auto(false);
        c.evaluate(Measurements { reply_success: 0.1, jitter_us: 50_000, blocked_ms_avg: 100 });
        assert_eq!(c.runahead_us(), 2000);
        assert_eq!(c.recv_timeout_ms(), 10);
    }

    #[test]
    fn jitter_above_runahead_forces_it_up() {
        let mut c = Controller::new();
        c.set_fixed(500, 8);
        c.evaluate(Measurements { reply_success: 1.0, jitter_us: 3000, blocked_ms_avg: 0 });
        assert!(c.runahead_us() >= 3000);
    }

    #[test]
    fn runahead_never_exceeds_clamp() {
        let mut c = Controller::new();
        c.set_fixed(RUNAHEAD_MAX_US, RECV_TIMEOUT_MAX_MS);
        for _ in 0..5 {
            c.evaluate(Measurements { reply_success: 0.1, jitter_us: 0, blocked_ms_avg: 200 });
        }
        assert_eq!(c.runahead_us(), RUNAHEAD_MAX_US);
        assert_eq!(c.recv_timeout_ms(), RECV_TIMEOUT_MAX_MS);
    }
}
