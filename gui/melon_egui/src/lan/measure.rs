//! What the link is measured to be doing, and the pace that follows from it.

use super::*;

// -- link measurement --------------------------------------------------------

/// What the link is doing, in numbers the UI can show and the transport can act
/// on.
///
/// Every field is atomic because it is written by the receive thread and the
/// emulation thread and read by the UI thread, none of which should ever wait
/// on another to draw a label.
#[derive(Default)]
pub(crate) struct Measurements {
    /// Smoothed round trip, in microseconds.
    pub(crate) rtt_us: AtomicU64,
    /// Smoothed absolute variation in round trip, in microseconds.
    pub(crate) jitter_us: AtomicU64,
    /// Datagrams sent and received, and frames within them.
    pub(crate) datagrams_sent: AtomicU64,
    pub(crate) datagrams_received: AtomicU64,
    pub(crate) frames_sent: AtomicU64,
    pub(crate) frames_received: AtomicU64,
    /// Datagrams discarded because their sequence number had been seen — the
    /// redundancy of [`Tuning::reply_copies`] doing its job.
    pub(crate) duplicates_dropped: AtomicU64,
    /// Rounds the host asked for and fully collected.
    pub(crate) rounds_answered: AtomicU64,
    /// Rounds that timed out with at least one client unheard. This is the
    /// number that becomes a communication error in the game.
    pub(crate) rounds_timed_out: AtomicU64,
    /// Replies discarded for arriving outside the staleness window.
    pub(crate) stale_replies: AtomicU64,
    /// Smoothed time the host actually spent waiting per round, in
    /// microseconds. What [`LinkPace`] is computed from.
    pub(crate) round_wait_us: AtomicU64,
}

/// A snapshot of [`Measurements`], for the diagnostics pane.
#[derive(Clone, Copy, Default, Debug)]
pub struct LinkStats {
    pub rtt_ms: f32,
    pub jitter_ms: f32,
    pub budget_ms: f32,
    pub datagrams_sent: u64,
    pub datagrams_received: u64,
    pub frames_sent: u64,
    pub frames_received: u64,
    pub duplicates_dropped: u64,
    pub rounds_answered: u64,
    pub rounds_timed_out: u64,
    pub stale_replies: u64,
    /// The frame rate the link can sustain, as [`LinkPace`] reports it.
    pub sustainable_fps: f32,
    /// Whether the console has switched its wireless on (`MP_Begin`). A link
    /// that is connected but not live is a cart that has not reached its
    /// multiplayer menu yet, which is worth being able to tell apart from a
    /// link that is silent because it is broken.
    pub wireless_on: bool,
}

impl LinkStats {
    /// The fraction of rounds that completed, or `None` before any round has
    /// been attempted.
    ///
    /// This is the single number that says whether the link is working: a game
    /// reports a communication error when this falls, and nothing else here
    /// predicts it as directly.
    #[must_use]
    pub fn round_success(&self) -> Option<f32> {
        let attempted = self.rounds_answered + self.rounds_timed_out;
        (attempted > 0).then(|| self.rounds_answered as f32 / attempted as f32)
    }
}

/// The emulated frame rate the link can sustain, shared with the front end.
///
/// Stored as millihertz in an atomic so the UI thread can read it without a
/// lock, and so the emulation thread can write it from inside a `Host`
/// callback — which is to say from inside `run_frame`, where taking a lock the
/// UI thread might hold would deadlock the console.
#[derive(Clone, Default)]
pub struct LinkPace(Arc<AtomicU32>);

/// The DS's own video frame rate, `33_513_982 / 560_190` Hz.
pub(crate) const NATIVE_FPS: f64 = 59.826_098;

impl LinkPace {
    /// The frame rate to pace at, never above the DS's own and never so low the
    /// window feels dead.
    ///
    /// `NATIVE_FPS` until the first round has been timed, which is the right
    /// answer for a console that has not started talking yet.
    #[must_use]
    pub fn frame_rate(&self) -> f64 {
        match self.0.load(Ordering::Relaxed) {
            0 => NATIVE_FPS,
            millihertz => f64::from(millihertz) / 1000.0,
        }
    }

    /// Record how long a round took and update the sustainable rate.
    ///
    /// The console cannot issue frames faster than one per
    /// `native frame time + the wait a round costs`, because the wait happens
    /// *inside* the frame. Pacing to anything faster only builds a debt that is
    /// discharged as a burst of rounds, which is worse for the peer than
    /// running slow.
    pub(crate) fn observe(&self, round_wait: Duration) {
        let period = 1.0 / NATIVE_FPS + round_wait.as_secs_f64();
        let fps = (1.0 / period).clamp(5.0, NATIVE_FPS);
        // A round that completes instantly must not snap the rate back to
        // native while the next one is about to block again, so the estimate is
        // smoothed the same way the round trip is.
        let previous = self.frame_rate();
        let smoothed = previous.mul_add(0.875, fps * 0.125);
        self.0.store((smoothed * 1000.0) as u32, Ordering::Relaxed);
    }
}

impl Measurements {
    /// Fold one round-trip sample into the smoothed estimates.
    ///
    /// The same exponentially weighted estimator RFC 6298 §2.3 specifies for
    /// TCP, with its 1/8 and 1/4 gains: it is well understood, cheap, and
    /// reacts to a VPN's step change in latency within about a second at
    /// [`PING_INTERVAL`].
    pub(crate) fn observe_rtt(&self, sample: Duration) {
        let sample = sample.as_micros().min(u128::from(u64::MAX)) as u64;
        let previous = self.rtt_us.load(Ordering::Relaxed);
        if previous == 0 {
            self.rtt_us.store(sample, Ordering::Relaxed);
            self.jitter_us.store(sample / 2, Ordering::Relaxed);
            return;
        }
        let deviation = previous.abs_diff(sample);
        let jitter = self.jitter_us.load(Ordering::Relaxed);
        self.jitter_us.store((jitter * 3 + deviation) / 4, Ordering::Relaxed);
        self.rtt_us.store((previous * 7 + sample) / 8, Ordering::Relaxed);
    }

    pub(crate) fn observe_round_wait(&self, waited: Duration) {
        let sample = waited.as_micros().min(u128::from(u64::MAX)) as u64;
        let previous = self.round_wait_us.load(Ordering::Relaxed);
        let smoothed = if previous == 0 { sample } else { (previous * 7 + sample) / 8 };
        self.round_wait_us.store(smoothed, Ordering::Relaxed);
    }
}
