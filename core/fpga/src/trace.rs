//! Waveform recording, as a simulator's wave window sees it: per signal,
//! the list of value changes stamped with the FPGA clock count.
//!
//! ```text
//!  core posedge #t -> Trace::record(values)  (only changes are stored)
//!  idle clocks skipped by a core            -> Trace::advance(n)
//!  viewer: value_at(signal, t), changes in [t0, t1]
//! ```
//!
//! Each signal keeps at most [`Trace::CAPACITY`] changes, so a busy
//! signal (a CPU's PC) covers a shorter past than a quiet one.

use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct TraceSignal {
    pub name: String,
    pub width: u32,
    /// (clock, value), oldest first.
    pub changes: VecDeque<(u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct Trace {
    pub signals: Vec<TraceSignal>,
    /// FPGA clocks since tracing started.
    pub now: u64,
}

impl Trace {
    pub const CAPACITY: usize = 4096;

    /// Starts a trace of `signals` (name, width) at clock 0.
    pub fn new(signals: impl IntoIterator<Item = (String, u32)>) -> Self {
        Self {
            signals: signals
                .into_iter()
                .map(|(name, width)| TraceSignal { name, width, changes: VecDeque::new() })
                .collect(),
            now: 0,
        }
    }

    /// Stores the values at the current clock, in `signals` order.
    pub fn record(&mut self, values: impl IntoIterator<Item = u64>) {
        let now = self.now;
        for (sig, v) in self.signals.iter_mut().zip(values) {
            if sig.changes.back().is_none_or(|&(_, last)| last != v) {
                if sig.changes.len() == Self::CAPACITY {
                    sig.changes.pop_front();
                }
                sig.changes.push_back((now, v));
            }
        }
    }

    /// `clocks` edges passed without a change.
    pub const fn advance(&mut self, clocks: u64) {
        self.now += clocks;
    }

    /// The value of `signal` at clock `t` (`None` before its first sample).
    pub fn value_at(&self, signal: usize, t: u64) -> Option<u64> {
        let changes = &self.signals.get(signal)?.changes;
        let i = changes.partition_point(|&(time, _)| time <= t);
        i.checked_sub(1).map(|i| changes[i].1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changes_are_kept() {
        let mut t = Trace::new([("a".to_owned(), 1), ("b".to_owned(), 8)]);
        for v in [0, 0, 1, 1, 0] {
            t.record([v, 7]);
            t.advance(1);
        }
        assert_eq!(t.signals[0].changes, [(0, 0), (2, 1), (4, 0)]);
        assert_eq!(t.signals[1].changes, [(0, 7)]);
        assert_eq!(t.value_at(0, 3), Some(1));
        assert_eq!(t.value_at(0, 4), Some(0));
    }
}
