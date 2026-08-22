//! One side's inbox for a class of frames.

use super::*;

// -- frame queues ------------------------------------------------------------

/// One side's inbox for a class of frames.
#[derive(Default)]
pub(crate) struct Queue {
    pub(crate) frames: Mutex<VecDeque<(Instant, Frame)>>,
    pub(crate) arrived: Condvar,
}

impl Queue {
    pub(crate) fn push(&self, frame: Frame) {
        let mut frames = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        if frames.len() >= QUEUE_CAPACITY {
            // Anything this old cannot answer a round that is still open, so it
            // goes first — that is the whole of the room-making, when there is
            // any to be had. `melonds::lan` skips this step and pops the front
            // unconditionally, which under a jitter burst throws away frames
            // that are still wanted while stale ones sit behind them.
            while frames.front().is_some_and(|(at, _)| at.elapsed() > STALE_FRAME_AGE) {
                frames.pop_front();
            }
            // Still full: everything queued is recent, so this is genuine
            // overload and the oldest is the least bad thing to lose.
            if frames.len() >= QUEUE_CAPACITY {
                frames.pop_front();
            }
        }
        frames.push_back((Instant::now(), frame));
        self.arrived.notify_all();
    }

    /// Take the first frame `wanted` accepts, waiting up to `timeout` for one.
    ///
    /// `timeout` of `None` does not wait at all, which is what the non-blocking
    /// `mp_recv_packet` needs.
    pub(crate) fn pop<F>(&self, timeout: Option<Duration>, wanted: F) -> Option<Frame>
    where
        F: Fn(&Frame) -> bool,
    {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut frames = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(at) = frames.iter().position(|(_, frame)| wanted(frame)) {
                return frames.remove(at).map(|(_, frame)| frame);
            }
            // No deadline means a non-blocking poll: the queue held nothing
            // wanted, and that is the answer.
            let remaining = deadline?.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let (guard, _) =
                self.arrived.wait_timeout(frames, remaining).unwrap_or_else(|e| e.into_inner());
            frames = guard;
        }
    }

    pub(crate) fn clear(&self) {
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}
