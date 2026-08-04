//! Stylus sample buffering between pointer sampling and emulation.
//!
//! The front end samples the host pointer once per repaint, but the emulator
//! consumes touch state once per emulated NDS frame, and the two rates are
//! unrelated: a 165 Hz display produces roughly three pointer samples per
//! emulated frame, while a host that briefly falls behind emulates several
//! frames back-to-back for a single sample.
//!
//! [`StylusQueue`] decouples the two. Samples are pushed in the order the
//! pointer produced them and taken one per emulated frame, with stale
//! positions collapsed away so a fast drag never accumulates latency: when
//! only one frame is left to emulate, the newest position wins. This matches
//! melonDS, whose emulation thread reads the latest coordinates written by the
//! Qt mouse handler rather than replaying a motion path.
//!
//! GBATEK "DS Touch Screen Controller (TSC)":
//! <https://problemkaputt.de/gbatek.htm#dstouchscreencontrollertsc>

use std::collections::VecDeque;

/// One stylus state: native bottom-screen coordinates while the pen is down,
/// or `None` for pen-up.
pub type TouchSample = Option<(usize, usize)>;

/// FIFO of stylus states waiting to be handed to the emulator.
#[derive(Debug, Default)]
pub struct StylusQueue {
    samples: VecDeque<TouchSample>,
    /// Last value handed to [`Self::push`], used to drop repeats. Holding the
    /// pointer still would otherwise enqueue one identical sample per repaint.
    last_pushed: Option<TouchSample>,
}

impl StylusQueue {
    /// Enqueues a stylus state. Consecutive duplicates are dropped, so a held
    /// (or absent) pointer costs nothing.
    pub fn push(&mut self, sample: TouchSample) {
        if self.last_pushed == Some(sample) {
            return;
        }
        self.last_pushed = Some(sample);
        self.samples.push_back(sample);
    }

    /// Takes the state to apply before the next emulated frame, given how many
    /// frames (including that one) are still to be emulated in this batch.
    ///
    /// Positions queued beyond what the remaining frames can consume are
    /// discarded oldest-first, so the emulator always ends the batch on the
    /// newest position instead of trailing behind by a growing backlog. A
    /// pen-up sample is never discarded this way: dropping it would silently
    /// merge a tap-lift-tap into one continuous drag.
    ///
    /// Returns `None` when nothing is queued, meaning the emulator should keep
    /// the state it already has (the TSC latches its coordinates).
    pub fn next_sample(&mut self, frames_remaining: usize) -> Option<TouchSample> {
        while self.samples.len() > frames_remaining.max(1)
            && matches!(self.samples.front(), Some(Some(_)))
        {
            self.samples.pop_front();
        }
        self.samples.pop_front()
    }

    /// Drops every queued sample, e.g. while emulation is paused and nothing
    /// is draining the queue. The duplicate filter is reset too, so the next
    /// push is enqueued even if it repeats the last pre-pause state.
    pub fn clear(&mut self) {
        self.samples.clear();
        self.last_pushed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_samples_are_coalesced() {
        let mut queue = StylusQueue::default();
        queue.push(Some((10, 10)));
        queue.push(Some((10, 10)));
        queue.push(Some((10, 10)));
        assert_eq!(queue.next_sample(1), Some(Some((10, 10))));
        assert_eq!(queue.next_sample(1), None);
    }

    #[test]
    fn single_remaining_frame_takes_the_newest_position() {
        let mut queue = StylusQueue::default();
        queue.push(Some((1, 1)));
        queue.push(Some((2, 2)));
        queue.push(Some((3, 3)));
        // The common case on a healthy host: one frame per repaint, so no
        // backlog may survive into the next one.
        assert_eq!(queue.next_sample(1), Some(Some((3, 3))));
        assert_eq!(queue.next_sample(1), None);
    }

    #[test]
    fn a_multi_frame_batch_traces_the_motion_path() {
        let mut queue = StylusQueue::default();
        queue.push(Some((1, 1)));
        queue.push(Some((2, 2)));
        queue.push(Some((3, 3)));
        assert_eq!(queue.next_sample(3), Some(Some((1, 1))));
        assert_eq!(queue.next_sample(2), Some(Some((2, 2))));
        assert_eq!(queue.next_sample(1), Some(Some((3, 3))));
    }

    #[test]
    fn pen_up_survives_collapsing() {
        let mut queue = StylusQueue::default();
        queue.push(Some((1, 1)));
        queue.push(None);
        queue.push(Some((9, 9)));
        // The release is delivered first even though only one frame is left;
        // the following press is then delivered by the next frame.
        assert_eq!(queue.next_sample(1), Some(None));
        assert_eq!(queue.next_sample(1), Some(Some((9, 9))));
    }

    #[test]
    fn clear_forgets_the_duplicate_filter() {
        let mut queue = StylusQueue::default();
        queue.push(Some((4, 4)));
        queue.clear();
        queue.push(Some((4, 4)));
        assert_eq!(queue.next_sample(1), Some(Some((4, 4))));
    }
}
