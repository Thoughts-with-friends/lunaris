//! Frame in, datagrams out — and the decision of which frames to skip.

use std::time::{Duration, Instant};

use super::{
    FRAME_PIXELS, MAX_DATAGRAM, Tuning,
    colour::to_565,
    tile::{self, TILE_COUNT},
    wire,
};

/// What one call to [`Encoder::encode`] produced, for the statistics pane.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameCost {
    /// Tiles that actually went out — changed, or refreshed.
    pub tiles: usize,
    /// Bytes on the wire, headers included.
    pub bytes: usize,
    /// Datagrams the frame took.
    pub datagrams: usize,
}

/// Turns a pair of framebuffers into independently applicable datagrams.
///
/// One per session. It keeps the last frame it **sent** so it can tell which
/// tiles moved; see the module documentation for why that reference does not
/// have to agree with what the client actually received.
pub struct Encoder {
    /// The frame as last sent, in RGB565.
    reference: Vec<u16>,
    /// Whether `reference` holds anything yet.
    primed: bool,
    /// Scratch, so a frame costs no allocation.
    tile_scratch: tile::Pixels,
    coded: Vec<u8>,
    frame: Vec<u16>,
    frame_seq: u32,
    /// Which slice of the rolling refresh the next frame paints.
    refresh_phase: u32,
    refresh_period: u32,
}

impl Encoder {
    /// Start a session. `refresh_period` comes from [`Tuning::refresh_period`].
    #[must_use]
    pub fn new(refresh_period: u8) -> Self {
        Self {
            reference: vec![0; FRAME_PIXELS],
            primed: false,
            tile_scratch: [0; tile::TILE_PIXELS],
            coded: Vec::with_capacity(tile::MAX_CODED),
            frame: vec![0; FRAME_PIXELS],
            frame_seq: 0,
            refresh_phase: 0,
            refresh_period: u32::from(refresh_period).max(1),
        }
    }

    /// Encode one frame into `out`, which is cleared first.
    ///
    /// Every element of `out` is a complete datagram the client can apply on
    /// its own.
    pub fn encode(&mut self, top: &[u32], bottom: &[u32], out: &mut Vec<Vec<u8>>) -> FrameCost {
        out.clear();
        self.quantise(top, bottom);

        self.frame_seq = self.frame_seq.wrapping_add(1);
        let phase = self.refresh_phase;
        self.refresh_phase = (self.refresh_phase + 1) % self.refresh_period;

        let mut cost = FrameCost::default();
        let mut datagram = wire::begin_video(self.frame_seq);
        let mut tiles = 0u16;

        for index in 0..TILE_COUNT {
            if !self.worth_sending(index, phase) {
                continue;
            }
            tile::gather(&self.frame, index, &mut self.tile_scratch);
            self.coded.clear();
            tile::pack(&self.tile_scratch, &mut self.coded);

            // 4 bytes of tile record: index, then coded length.
            if datagram.len() + 4 + self.coded.len() > MAX_DATAGRAM && tiles > 0 {
                Self::flush(&mut datagram, tiles, &mut cost, out);
                datagram = wire::begin_video(self.frame_seq);
                tiles = 0;
            }
            datagram.extend_from_slice(&(index as u16).to_le_bytes());
            datagram.extend_from_slice(&(self.coded.len() as u16).to_le_bytes());
            datagram.extend_from_slice(&self.coded);
            tiles += 1;
            cost.tiles += 1;
        }
        if tiles > 0 {
            Self::flush(&mut datagram, tiles, &mut cost, out);
        }

        std::mem::swap(&mut self.reference, &mut self.frame);
        self.primed = true;
        cost
    }

    /// Convert both screens into this frame's RGB565 buffer.
    fn quantise(&mut self, top: &[u32], bottom: &[u32]) {
        for (dst, src) in self.frame.iter_mut().zip(top.iter().chain(bottom.iter())) {
            *dst = to_565(*src);
        }
    }

    /// Whether tile `index` goes out this frame: because it changed, because it
    /// is this frame's turn in the rolling refresh, or because there is no
    /// reference yet.
    fn worth_sending(&self, index: usize, phase: u32) -> bool {
        !self.primed
            || index as u32 % self.refresh_period == phase
            || tile::differs(&self.frame, &self.reference, index)
    }

    fn flush(datagram: &mut Vec<u8>, tiles: u16, cost: &mut FrameCost, out: &mut Vec<Vec<u8>>) {
        wire::finish_video(datagram, tiles);
        cost.bytes += datagram.len();
        cost.datagrams += 1;
        out.push(std::mem::take(datagram));
    }
}

/// Decides which frames are worth sending.
///
/// # Why skipping costs nothing but smoothness
///
/// A skipped frame is not a delayed frame. Nothing is queued: a datagram goes
/// on the wire the instant it is encoded, and a frame that is not encoded
/// simply leaves its changes to accumulate into the next one that is. So the
/// path from a button press to the pixel that answers it is unchanged — the
/// picture merely arrives in fewer, larger steps.
///
/// That is what makes this the right lever. Lowering the *frame rate* keeps
/// the link responsive; the alternatives — compressing harder, or buffering —
/// cost picture quality or latency respectively.
///
/// # How the interval moves
///
/// It starts at [`Tuning::fastest_interval`]. Once a second the bytes actually
/// sent are compared against [`Tuning::max_bitrate_kbps`]: over budget and the
/// interval grows by one, comfortably under and it shrinks by one. One step a
/// second is deliberately slow — a rate that chases every burst would visibly
/// stutter, and the thing being measured is a link's sustained capacity rather
/// than its instantaneous one.
pub struct Pacer {
    interval: u32,
    fastest: u32,
    slowest: u32,
    budget_bytes_per_second: u64,
    /// Emulated frames since the last one that was sent.
    since_sent: u32,
    /// Bytes sent in the window that began at [`Self::window_started`].
    window_bytes: u64,
    window_started: Instant,
}

/// How long the bit rate is averaged over before the interval is adjusted.
const WINDOW: Duration = Duration::from_secs(1);

impl Pacer {
    #[must_use]
    pub fn new(tuning: &Tuning) -> Self {
        Self {
            interval: tuning.fastest_interval(),
            fastest: tuning.fastest_interval(),
            slowest: tuning.slowest_interval(),
            budget_bytes_per_second: u64::from(tuning.max_bitrate_kbps) * 1000 / 8,
            since_sent: u32::MAX,
            window_bytes: 0,
            window_started: Instant::now(),
        }
    }

    /// Whether this emulated frame's picture should be encoded.
    ///
    /// Called once per emulated frame; the caller encodes only when this says
    /// so. `since_sent` starts at `u32::MAX` so the very first frame always
    /// goes — a session that opened on a black screen for two frames would look
    /// like a failed connection.
    pub fn due(&mut self) -> bool {
        self.since_sent = self.since_sent.saturating_add(1);
        if self.since_sent < self.interval {
            return false;
        }
        self.since_sent = 0;
        true
    }

    /// Record what a sent frame cost, and adjust the interval if the window has
    /// closed.
    pub fn observe(&mut self, bytes: usize) {
        self.window_bytes += bytes as u64;
        let elapsed = self.window_started.elapsed();
        if elapsed < WINDOW {
            return;
        }
        let rate = (self.window_bytes as f64 / elapsed.as_secs_f64()) as u64;
        if rate > self.budget_bytes_per_second {
            self.interval = (self.interval + 1).min(self.slowest);
        } else if rate * 4 < self.budget_bytes_per_second * 3 {
            // Only back off the skipping when there is real room — a quarter of
            // the budget spare — so the interval does not oscillate around the
            // exact threshold.
            self.interval = self.interval.saturating_sub(1).max(self.fastest);
        }
        self.window_bytes = 0;
        self.window_started = Instant::now();
    }

    /// The frame rate the picture is currently being sent at.
    #[must_use]
    pub fn frames_per_second(&self) -> f32 {
        super::NATIVE_FPS as f32 / self.interval.max(1) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::{Encoder, Pacer};
    use crate::remote::{Tuning, tile::TILE_COUNT};

    fn flat_screen(value: u32) -> Vec<u32> {
        vec![value; crate::remote::SCREEN_WIDTH * crate::remote::SCREEN_HEIGHT]
    }

    #[test]
    fn the_first_frame_sends_every_tile() {
        let mut encoder = Encoder::new(8);
        let mut out = Vec::new();
        let cost = encoder.encode(&flat_screen(0x0011_2233), &flat_screen(0), &mut out);
        assert_eq!(cost.tiles, TILE_COUNT);
    }

    /// The point of the delta: a still picture costs only its refresh slice.
    #[test]
    fn a_still_picture_costs_only_the_rolling_refresh() {
        let (top, bottom) = (flat_screen(0x0011_2233), flat_screen(0x0044_5566));
        let mut encoder = Encoder::new(8);
        let mut out = Vec::new();
        encoder.encode(&top, &bottom, &mut out);
        let repeat = encoder.encode(&top, &bottom, &mut out);
        assert!(
            repeat.tiles <= TILE_COUNT / 8 + 1,
            "an unchanged frame sent {} tiles, more than one refresh slice",
            repeat.tiles
        );
    }

    #[test]
    fn the_pacer_sends_the_first_frame_then_every_interval() {
        let tuning = Tuning { max_video_fps: 30, ..Tuning::default() };
        let mut pacer = Pacer::new(&tuning);
        assert!(pacer.due(), "the first frame must always go");
        assert!(!pacer.due(), "30 fps skips every other emulated frame");
        assert!(pacer.due());
        assert!(!pacer.due());
    }

    /// A link that cannot carry the picture must end up sending fewer frames,
    /// not a backlog of them.
    #[test]
    fn the_pacer_slows_down_when_the_budget_is_exceeded() {
        let tuning = Tuning { max_bitrate_kbps: 1_000, ..Tuning::default() };
        let mut pacer = Pacer::new(&tuning);
        let before = pacer.frames_per_second();
        // Well over 125 KB/s, reported after the window has closed.
        std::thread::sleep(super::WINDOW);
        pacer.observe(400_000);
        assert!(
            pacer.frames_per_second() < before,
            "the rate stayed at {before} despite four times the budget"
        );
    }

    #[test]
    fn the_pacer_speeds_back_up_when_the_link_is_quiet() {
        let tuning = Tuning { max_bitrate_kbps: 1_000, ..Tuning::default() };
        let mut pacer = Pacer::new(&tuning);
        std::thread::sleep(super::WINDOW);
        pacer.observe(400_000);
        let slowed = pacer.frames_per_second();
        std::thread::sleep(super::WINDOW);
        pacer.observe(1_000);
        assert!(
            pacer.frames_per_second() > slowed,
            "the rate stayed at {slowed} with the link almost idle"
        );
    }

    /// However bad the link, the picture must keep moving.
    #[test]
    fn the_pacer_never_falls_below_the_floor() {
        let tuning = Tuning { min_video_fps: 10, max_bitrate_kbps: 200, ..Tuning::default() };
        let mut pacer = Pacer::new(&tuning);
        for _ in 0..8 {
            std::thread::sleep(super::WINDOW);
            pacer.observe(10_000_000);
        }
        assert!(
            pacer.frames_per_second() >= 9.0,
            "the rate collapsed to {}",
            pacer.frames_per_second()
        );
    }
}
