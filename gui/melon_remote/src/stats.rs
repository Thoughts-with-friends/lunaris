//! What a session is doing, in numbers the pane can show.
//!
//! Every counter is atomic because it is written by the receive thread and the
//! console's thread and read by the UI thread, none of which should ever wait
//! on another to draw a label.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

/// How long the other end may go without answering a ping before it is shown
/// as not ready: four missed probes at [`super::session::PING_INTERVAL`], so
/// one lost datagram never flickers the status red.
pub const READY_WITHIN: Duration = Duration::from_secs(2);

/// A snapshot of [`Counters`].
#[derive(Clone, Copy, Default, Debug)]
pub struct RemoteStats {
    pub rtt_ms: f32,
    /// How long since the other end last answered a ping, or `None` if it
    /// never has. Measured, not assumed: a session whose peer has gone away
    /// keeps its socket, so "the session exists" says nothing about the peer.
    pub silent_ms: Option<f32>,
    /// Frames sent, or rebuilt.
    pub frames: u64,
    /// Frames the pacer decided not to send. See [`super::encoder::Pacer`].
    pub frames_skipped: u64,
    /// The rate the picture is currently going out at.
    pub video_fps: f32,
    pub video_datagrams: u64,
    pub video_bytes: u64,
    /// The rate the sound travels at, which is not the console's.
    pub audio_rate: u32,
    pub audio_pairs: u64,
    /// Sample pairs dropped to stop the sound drifting behind the picture.
    pub audio_dropped: u64,
    pub inputs: u64,
    /// Datagrams refused as out of order or malformed.
    pub discarded: u64,
    /// The most recent frame's cost, so the codec's work is visible.
    pub last_frame_tiles: usize,
    pub last_frame_bytes: usize,
}

impl RemoteStats {
    /// Whether the other end has answered a ping within [`READY_WITHIN`].
    #[must_use]
    pub fn peer_ready(&self) -> bool {
        self.silent_ms.is_some_and(|ms| ms <= READY_WITHIN.as_secs_f32() * 1000.0)
    }

    /// The video bit rate implied by the last frame at the rate it is going out
    /// at — which is what actually crosses the link, rather than what a frame
    /// would cost at 59.83 fps.
    #[must_use]
    pub fn video_megabits_per_second(&self) -> f32 {
        self.last_frame_bytes as f32 * 8.0 * self.video_fps / 1_000_000.0
    }

    /// The sound's bit rate, from the rate it travels at: stereo `i16`.
    #[must_use]
    pub fn audio_megabits_per_second(&self) -> f32 {
        self.audio_rate as f32 * 2.0 * 16.0 / 1_000_000.0
    }

    /// What the sound would have cost untouched, so the saving is visible
    /// rather than merely claimed.
    #[must_use]
    pub fn audio_megabits_per_second_raw() -> f32 {
        super::CONSOLE_SAMPLE_RATE as f32 * 2.0 * 16.0 / 1_000_000.0
    }
}

#[derive(Default)]
pub struct Counters {
    pub frames: AtomicU64,
    pub frames_skipped: AtomicU64,
    pub video_datagrams: AtomicU64,
    pub video_bytes: AtomicU64,
    pub audio_pairs: AtomicU64,
    pub audio_dropped: AtomicU64,
    pub inputs: AtomicU64,
    pub discarded: AtomicU64,
    pub rtt_us: AtomicU64,
    pub audio_rate: AtomicU64,
    pub video_millifps: AtomicU64,
    pub last_frame_tiles: AtomicU64,
    pub last_frame_bytes: AtomicU64,
    /// When the last `Pong` arrived, as microseconds since the session began
    /// plus one; zero means never. Relative to an [`Instant`] rather than the
    /// wall clock so a clock change cannot make a silent peer look alive.
    pub last_pong_us: AtomicU64,
}

impl Counters {
    /// The numbers as they stand, `epoch` being when the session began.
    #[must_use]
    pub fn snapshot(&self, epoch: Instant) -> RemoteStats {
        let load = |field: &AtomicU64| field.load(Ordering::Relaxed);
        let silent_ms = match load(&self.last_pong_us) {
            0 => None,
            heard => {
                let now = epoch.elapsed().as_micros() as u64;
                Some(now.saturating_sub(heard - 1) as f32 / 1000.0)
            }
        };
        RemoteStats {
            rtt_ms: load(&self.rtt_us) as f32 / 1000.0,
            silent_ms,
            frames: load(&self.frames),
            frames_skipped: load(&self.frames_skipped),
            video_fps: load(&self.video_millifps) as f32 / 1000.0,
            video_datagrams: load(&self.video_datagrams),
            video_bytes: load(&self.video_bytes),
            audio_rate: load(&self.audio_rate) as u32,
            audio_pairs: load(&self.audio_pairs),
            audio_dropped: load(&self.audio_dropped),
            inputs: load(&self.inputs),
            discarded: load(&self.discarded),
            last_frame_tiles: load(&self.last_frame_tiles) as usize,
            last_frame_bytes: load(&self.last_frame_bytes) as usize,
        }
    }

    /// Fold a round-trip sample in, the same 1/8-gain estimator
    /// [`crate::lan`] uses.
    pub fn observe_rtt(&self, sample: Duration) {
        let sample = sample.as_micros().min(u128::from(u64::MAX)) as u64;
        let previous = self.rtt_us.load(Ordering::Relaxed);
        let smoothed = if previous == 0 { sample } else { (previous * 7 + sample) / 8 };
        self.rtt_us.store(smoothed, Ordering::Relaxed);
    }

    /// Record that the other end just answered a ping.
    pub fn heard_from_peer(&self, epoch: Instant) {
        let since = epoch.elapsed().as_micros().min(u128::from(u64::MAX - 1)) as u64;
        self.last_pong_us.store(since + 1, Ordering::Relaxed);
    }

    pub fn bump(&self, field: &AtomicU64, by: u64) {
        field.fetch_add(by, Ordering::Relaxed);
    }

    pub fn set(&self, field: &AtomicU64, value: u64) {
        field.store(value, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Counters, READY_WITHIN, RemoteStats};

    /// Never heard from is not ready; just heard from is; heard from long ago
    /// is not again.
    #[test]
    fn readiness_follows_the_last_pong() {
        let counters = Counters::default();
        let epoch = Instant::now();
        assert!(!counters.snapshot(epoch).peer_ready(), "no pong yet");
        counters.heard_from_peer(epoch);
        assert!(counters.snapshot(epoch).peer_ready());

        // Heard at the very start of a session that began READY_WITHIN + 1 s
        // ago, and not since.
        let long_ago = epoch - READY_WITHIN - Duration::from_secs(1);
        let stale = Counters::default();
        stale.last_pong_us.store(1, std::sync::atomic::Ordering::Relaxed);
        let snapshot = stale.snapshot(long_ago);
        assert!(!snapshot.peer_ready(), "silent for {:?} ms", snapshot.silent_ms);
    }

    /// The whole point of the transport rate: the saving has to be real, and
    /// visible in the numbers the pane prints.
    #[test]
    fn halving_the_audio_rate_halves_its_bit_rate() {
        let full = RemoteStats { audio_rate: 48_000, ..RemoteStats::default() };
        let half = RemoteStats { audio_rate: 24_000, ..RemoteStats::default() };
        assert!((full.audio_megabits_per_second() - 1.536).abs() < 0.001);
        assert!((half.audio_megabits_per_second() - 0.768).abs() < 0.001);
        assert!(
            (RemoteStats::audio_megabits_per_second_raw() - full.audio_megabits_per_second()).abs()
                < 0.001
        );
    }

    /// The video figure has to use the rate frames are actually sent at, or
    /// skipping would not show up as a saving.
    #[test]
    fn the_video_rate_follows_the_frames_actually_sent() {
        let sixty =
            RemoteStats { last_frame_bytes: 10_000, video_fps: 60.0, ..RemoteStats::default() };
        let thirty = RemoteStats { video_fps: 30.0, ..sixty };
        assert!((sixty.video_megabits_per_second() - 4.8).abs() < 0.01);
        assert!((thirty.video_megabits_per_second() - 2.4).abs() < 0.01);
    }
}
