//! Remote Desktop mode: stream a console's picture out, take its input back.
//!
//! # Why this exists, and why tuning [`crate::lan`] could never have worked
//!
//! In LAN mode the *emulated wireless* crosses the network, and a DS wireless
//! round has to complete inside one emulated frame: the host sends CMD, the
//! client answers, the host reads the answers, all before the frame ends
//! (GBATEK, "DS Wifi ... Multiplay"). The round trip is therefore **inside** the
//! frame, so the best achievable frame rate is
//!
//! ```text
//! 1 / (16.7 ms + round trip)
//! ```
//!
//! A real session over a 16.9 ms VPN measured 29.6 fps — and `1/(16.7+16.9)` is
//! 29.8. That is not a tuning failure; it is the formula. No budget, no
//! redundancy and no clock trick can beat it, because the network is a term in
//! the denominator.
//!
//! Remote Desktop takes the network out of that denominator. **Both** consoles
//! run on the host machine, on the in-process airwaves of [`crate::mp`], where a
//! round costs microseconds and 59.83 fps is kept. What crosses the network is
//! the second console's *picture and sound* one way, and the remote player's
//! *buttons and stylus* the other — neither of which any emulated frame waits
//! for.
//!
//! ```text
//! host machine                                  remote machine
//! ┌───────────┬───────────┐                     ┌──────────────┐
//! │ instance1 ↔ instance2 │ ── video + audio ─→ │   display    │
//! │  (in-process wireless)│ ←──── input ─────── │ buttons/touch│
//! └───────────┴───────────┘                     └──────────────┘
//! ```
//!
//! Latency becomes *asymmetric*: the host player has none, and the remote
//! player sees `round trip + one frame` — about 34 ms at the link measured
//! above. That is worse than sitting at the host machine and far better than
//! 29.6 fps with two rounds in five failing.
//!
//! Because every console is on the host, **the host holds all of it**: saves,
//! savestates, cheats, instance directories. A client owns nothing and is asked
//! for nothing.
//!
//! # The video codec, and why it needs no acknowledgements
//!
//! Each screen is cut into 16×16 tiles ([`TILE`]). A tile is encoded whole,
//! in RGB565, with a PackBits run/literal coder — DS art is mostly flat colour,
//! which that suits. A frame sends the tiles that **changed**, plus the tiles
//! whose turn it is in a rolling refresh (see [`Tuning::refresh_period`]).
//!
//! Whole tiles are packed into MTU-sized datagrams, so **every datagram is
//! independently applicable**: a client that receives one applies its tiles and
//! is that much more correct, whatever else was lost. There is no reassembly,
//! no acknowledgement, no keyframe and no request-for-keyframe — a lost
//! datagram leaves a few tiles stale for at most one refresh period, and then
//! they are painted again by the rolling refresh. That is what makes this
//! usable on the sort of link that was breaking LAN mode.

use std::{
    collections::VecDeque,
    io,
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

/// One DS screen, as `melonds::SCREEN_WIDTH` / `SCREEN_HEIGHT`.
///
/// Repeated here rather than imported so this module builds without the
/// emulator core, which is what lets the codec be tested on any machine.
pub const SCREEN_WIDTH: usize = 256;
/// See [`SCREEN_WIDTH`].
pub const SCREEN_HEIGHT: usize = 192;

/// The side of a tile, in pixels.
///
/// 16 is the usual compromise: small enough that a moving sprite dirties only
/// the tiles it touches, large enough that the 4-byte per-tile record does not
/// dominate. It also divides both screen dimensions exactly, so there are no
/// partial tiles to special-case.
pub const TILE: usize = 16;

/// Tiles across one screen.
pub const TILES_X: usize = SCREEN_WIDTH / TILE;
/// Tiles down one screen.
pub const TILES_Y: usize = SCREEN_HEIGHT / TILE;
/// Tiles in one screen.
pub const TILES_PER_SCREEN: usize = TILES_X * TILES_Y;
/// Tiles in a frame — both screens, top first.
pub const TILE_COUNT: usize = TILES_PER_SCREEN * 2;
/// Pixels in one tile.
const TILE_PIXELS: usize = TILE * TILE;
/// Pixels in a frame, both screens.
const FRAME_PIXELS: usize = SCREEN_WIDTH * SCREEN_HEIGHT * 2;

/// Identifies this protocol's datagrams.
const MAGIC: &[u8; 4] = b"MRD1";

/// How large a datagram may grow.
///
/// Under a typical VPN's reduced MTU (WireGuard defaults to 1420) less its own
/// headers, so that a video datagram is never IP-fragmented — losing one
/// fragment would lose every tile in it, which is exactly the coupling this
/// codec exists to avoid.
const MAX_DATAGRAM: usize = 1200;

/// How often the host probes the link, in wall time.
const PING_INTERVAL: Duration = Duration::from_millis(500);

/// How many sample pairs one audio datagram carries.
///
/// 240 pairs is 5 ms at 48 kHz, so a lost datagram is a 5 ms gap — short enough
/// to be a click rather than a dropout, and small enough that the datagram
/// stays well inside [`MAX_DATAGRAM`].
const AUDIO_CHUNK_PAIRS: usize = 240;

/// What a datagram is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Client announcing itself.
    Hello = 0,
    /// Host accepting.
    Welcome = 1,
    /// Some of a frame's tiles. Independently applicable.
    Video = 2,
    /// A run of interleaved stereo samples.
    Audio = 3,
    /// The remote player's buttons and stylus, as a whole state.
    Input = 4,
    /// Latency probe, carrying the sender's wall clock in microseconds.
    Ping = 5,
    /// Latency probe echo, carrying the `Ping`'s value untouched.
    Pong = 6,
}

impl Kind {
    const fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Hello),
            1 => Some(Self::Welcome),
            2 => Some(Self::Video),
            3 => Some(Self::Audio),
            4 => Some(Self::Input),
            5 => Some(Self::Ping),
            6 => Some(Self::Pong),
            _ => None,
        }
    }
}

// -- tuning ------------------------------------------------------------------

/// The knobs behind Remote Desktop mode, persisted in the instance's
/// `settings.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Tuning {
    /// How many frames a complete rolling refresh takes.
    ///
    /// This is the whole of the loss recovery: a tile that was sent and lost is
    /// repainted within this many frames whatever else happens. Lower recovers
    /// faster and costs bandwidth on a still picture; higher is cheaper and
    /// leaves a dropped tile visible for longer. 8 frames is 134 ms.
    pub refresh_period: u8,
    /// Whether the remote player hears the console.
    pub audio: bool,
    /// How much audio may be in flight before the oldest is dropped, in
    /// milliseconds.
    ///
    /// Audio that is queued is audio that is late, and a queue that is never
    /// trimmed grows without bound — the sound drifts further behind the
    /// picture for as long as the session lasts. Dropping is audible once;
    /// drifting is audible forever.
    pub max_audio_lag_ms: u16,
    /// The UDP port the host binds and the client connects to by default.
    pub port: u16,
}

impl Default for Tuning {
    fn default() -> Self {
        Self { refresh_period: 8, audio: true, max_audio_lag_ms: 120, port: 7065 }
    }
}

impl Tuning {
    /// Clamp every field to something the protocol can honour, so a hand-edited
    /// `settings.json` cannot put a session in a state the UI could not produce.
    pub fn normalize(&mut self) {
        self.refresh_period = self.refresh_period.clamp(1, 60);
        self.max_audio_lag_ms = self.max_audio_lag_ms.clamp(20, 1000);
        if self.port == 0 {
            self.port = Self::default().port;
        }
    }
}

// -- colour ------------------------------------------------------------------

/// Pack one framebuffer pixel into RGB565.
///
/// The core hands pixels over as `0x00RRGGBB` in a `u32` — the same order
/// `crate::app::to_image` reads them in, which is where this shift pattern comes
/// from. Deriving it rather than rewriting it matters: an independently written
/// channel order that swapped red and blue would look plausible until somebody
/// noticed the skin tones were wrong.
///
/// 565 rather than the framebuffer's 888 halves the bytes and costs almost
/// nothing here: the DS's own output is 6 bits per channel (GBATEK, "DS Video
/// BG Modes"), so green is exact and red and blue lose one bit each.
#[must_use]
const fn to_565(pixel: u32) -> u16 {
    let r = ((pixel >> 16) & 0xFF) as u16;
    let g = ((pixel >> 8) & 0xFF) as u16;
    let b = (pixel & 0xFF) as u16;
    ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3)
}

/// Unpack RGB565 back into the framebuffer's `0x00RRGGBB`.
///
/// The low bits are filled from the high ones rather than with zeros, so that
/// full white stays full white — a plain shift would turn `0xFF` into `0xF8`
/// and tint every bright area very slightly dark.
#[must_use]
const fn from_565(pixel: u16) -> u32 {
    let r = ((pixel >> 11) & 0x1F) as u32;
    let g = ((pixel >> 5) & 0x3F) as u32;
    let b = (pixel & 0x1F) as u32;
    let r = (r << 3) | (r >> 2);
    let g = (g << 2) | (g >> 4);
    let b = (b << 3) | (b >> 2);
    (r << 16) | (g << 8) | b
}

// -- tile coding ---------------------------------------------------------------

/// Where tile `index` starts in a frame's pixel buffer, and which screen it is
/// on.
///
/// Tiles are numbered top screen first, then bottom; within a screen, left to
/// right then top to bottom.
const fn tile_origin(index: usize) -> usize {
    let screen = index / TILES_PER_SCREEN;
    let within = index % TILES_PER_SCREEN;
    let ty = within / TILES_X;
    let tx = within % TILES_X;
    screen * (SCREEN_WIDTH * SCREEN_HEIGHT) + ty * TILE * SCREEN_WIDTH + tx * TILE
}

/// Whether tile `index` differs between two frame buffers.
fn tile_differs(a: &[u16], b: &[u16], index: usize) -> bool {
    let origin = tile_origin(index);
    (0..TILE).any(|row| {
        let at = origin + row * SCREEN_WIDTH;
        a[at..at + TILE] != b[at..at + TILE]
    })
}

/// Copy one tile out of a frame into a flat 256-pixel scratch.
fn gather_tile(frame: &[u16], index: usize, out: &mut [u16; TILE_PIXELS]) {
    let origin = tile_origin(index);
    for row in 0..TILE {
        let at = origin + row * SCREEN_WIDTH;
        out[row * TILE..(row + 1) * TILE].copy_from_slice(&frame[at..at + TILE]);
    }
}

/// Write one tile back into a frame.
fn scatter_tile(frame: &mut [u16], index: usize, tile: &[u16; TILE_PIXELS]) {
    let origin = tile_origin(index);
    for row in 0..TILE {
        let at = origin + row * SCREEN_WIDTH;
        frame[at..at + TILE].copy_from_slice(&tile[row * TILE..(row + 1) * TILE]);
    }
}

/// The largest run a single token can express.
const MAX_RUN: usize = 129;
/// The most literals a single token can introduce.
const MAX_LITERAL: usize = 128;

/// PackBits-style run/literal coding over 16-bit pixels.
///
/// A token below `0x80` introduces `token + 1` literal pixels; a token at or
/// above it repeats the single pixel that follows `token - 0x80 + 2` times. The
/// worst case — 256 pixels, none repeating — is 2 tokens plus 512 bytes, two
/// bytes over raw, which is what makes this safe to use unconditionally rather
/// than needing a "store raw" fallback.
fn pack_tile(tile: &[u16; TILE_PIXELS], out: &mut Vec<u8>) {
    let mut at = 0;
    while at < TILE_PIXELS {
        // How far the pixel at `at` repeats.
        let mut run = 1;
        while at + run < TILE_PIXELS && tile[at + run] == tile[at] && run < MAX_RUN {
            run += 1;
        }
        if run >= 2 {
            out.push(0x80 | (run - 2) as u8);
            out.extend_from_slice(&tile[at].to_le_bytes());
            at += run;
            continue;
        }
        // No run here, so gather literals until one starts. A run of two is not
        // worth breaking a literal for (it costs the same), so literals end
        // only on a run of three or more.
        let start = at;
        while at < TILE_PIXELS && at - start < MAX_LITERAL {
            let ahead = TILE_PIXELS - at;
            if ahead >= 3 && tile[at] == tile[at + 1] && tile[at] == tile[at + 2] {
                break;
            }
            at += 1;
        }
        out.push((at - start - 1) as u8);
        for pixel in &tile[start..at] {
            out.extend_from_slice(&pixel.to_le_bytes());
        }
    }
}

/// Undo [`pack_tile`], returning how many bytes were consumed.
///
/// `None` for anything malformed. A datagram from a different version — or from
/// nowhere in particular, since a UDP port takes whatever is sent to it — must
/// be refused rather than trusted with an index.
fn unpack_tile(bytes: &[u8], out: &mut [u16; TILE_PIXELS]) -> Option<usize> {
    let mut read = 0;
    let mut written = 0;
    while written < TILE_PIXELS {
        let token = *bytes.get(read)?;
        read += 1;
        if token & 0x80 == 0 {
            let count = usize::from(token) + 1;
            if written + count > TILE_PIXELS || read + count * 2 > bytes.len() {
                return None;
            }
            for slot in &mut out[written..written + count] {
                *slot = u16::from_le_bytes([bytes[read], bytes[read + 1]]);
                read += 2;
            }
            written += count;
        } else {
            let count = usize::from(token & 0x7F) + 2;
            if written + count > TILE_PIXELS || read + 2 > bytes.len() {
                return None;
            }
            let pixel = u16::from_le_bytes([bytes[read], bytes[read + 1]]);
            read += 2;
            out[written..written + count].fill(pixel);
            written += count;
        }
    }
    Some(read)
}

// -- the encoder ---------------------------------------------------------------

/// Turns a pair of framebuffers into independently applicable datagrams.
///
/// One encoder per session. It keeps the last frame it sent so it can tell
/// which tiles moved; see the module documentation for why that reference does
/// not have to agree with what the client actually received.
pub struct Encoder {
    /// The frame as last sent, in RGB565.
    reference: Vec<u16>,
    /// Whether `reference` holds anything yet.
    primed: bool,
    /// Scratch, so a frame costs no allocation.
    scratch: [u16; TILE_PIXELS],
    tile_bytes: Vec<u8>,
    frame_seq: u32,
    /// Which slice of the rolling refresh this frame paints.
    refresh_phase: u32,
    refresh_period: u32,
}

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

impl Encoder {
    /// Start a session. `refresh_period` comes from [`Tuning::refresh_period`].
    #[must_use]
    pub fn new(refresh_period: u8) -> Self {
        Self {
            reference: vec![0; FRAME_PIXELS],
            primed: false,
            scratch: [0; TILE_PIXELS],
            tile_bytes: Vec::with_capacity(TILE_PIXELS * 2 + 2),
            frame_seq: 0,
            refresh_phase: 0,
            refresh_period: u32::from(refresh_period).max(1),
        }
    }

    /// Encode one frame into `out`, which is cleared first.
    ///
    /// Every element of `out` is a complete datagram that the client can apply
    /// on its own.
    pub fn encode(&mut self, top: &[u32], bottom: &[u32], out: &mut Vec<Vec<u8>>) -> FrameCost {
        out.clear();
        // Both screens into one buffer, so a tile index addresses the frame
        // rather than a screen and the rolling refresh sweeps both together.
        let mut frame = vec![0u16; FRAME_PIXELS];
        for (dst, src) in frame.iter_mut().zip(top.iter().chain(bottom.iter())) {
            *dst = to_565(*src);
        }

        self.frame_seq = self.frame_seq.wrapping_add(1);
        let phase = self.refresh_phase;
        self.refresh_phase = (self.refresh_phase + 1) % self.refresh_period;

        let mut cost = FrameCost::default();
        let mut datagram = self.begin_datagram();
        let mut tiles_in_datagram = 0u16;

        for index in 0..TILE_COUNT {
            // The first frame of a session has no reference worth trusting, so
            // everything goes; after that it is "changed, or its turn".
            let refreshed = index as u32 % self.refresh_period == phase;
            if self.primed && !refreshed && !tile_differs(&frame, &self.reference, index) {
                continue;
            }

            gather_tile(&frame, index, &mut self.scratch);
            self.tile_bytes.clear();
            pack_tile(&self.scratch, &mut self.tile_bytes);

            // 4 bytes of tile record header: index, then length.
            let needed = 4 + self.tile_bytes.len();
            if datagram.len() + needed > MAX_DATAGRAM && tiles_in_datagram > 0 {
                finish_datagram(&mut datagram, tiles_in_datagram);
                cost.bytes += datagram.len();
                cost.datagrams += 1;
                out.push(std::mem::take(&mut datagram));
                datagram = self.begin_datagram();
                tiles_in_datagram = 0;
            }
            datagram.extend_from_slice(&(index as u16).to_le_bytes());
            datagram.extend_from_slice(&(self.tile_bytes.len() as u16).to_le_bytes());
            datagram.extend_from_slice(&self.tile_bytes);
            tiles_in_datagram += 1;
            cost.tiles += 1;
        }

        if tiles_in_datagram > 0 {
            finish_datagram(&mut datagram, tiles_in_datagram);
            cost.bytes += datagram.len();
            cost.datagrams += 1;
            out.push(datagram);
        }

        self.reference = frame;
        self.primed = true;
        cost
    }

    fn begin_datagram(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_DATAGRAM);
        bytes.extend_from_slice(MAGIC);
        bytes.push(Kind::Video as u8);
        bytes.extend_from_slice(&self.frame_seq.to_le_bytes());
        // Tile count, filled in by `finish_datagram` once it is known.
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }
}

/// Header length of a video datagram: magic, kind, frame sequence, tile count.
const VIDEO_HEADER: usize = 4 + 1 + 4 + 2;

fn finish_datagram(bytes: &mut [u8], tiles: u16) {
    bytes[9..11].copy_from_slice(&tiles.to_le_bytes());
}

// -- the decoder ---------------------------------------------------------------

/// Rebuilds the picture from whatever datagrams arrive.
pub struct Decoder {
    pixels: Vec<u16>,
    /// The newest frame any applied tile came from. Tiles from older frames are
    /// refused, so a datagram that overtook another cannot paint stale pixels
    /// over fresh ones.
    newest_seq: u32,
    /// Set whenever a tile is applied, so the front end only rebuilds its
    /// textures when there is something new to show.
    dirty: bool,
    tiles_applied: u64,
    frames_seen: u64,
    /// Datagrams refused for being older than what is already shown.
    reordered: u64,
    malformed: u64,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pixels: vec![0; FRAME_PIXELS],
            newest_seq: 0,
            dirty: false,
            tiles_applied: 0,
            frames_seen: 0,
            reordered: 0,
            malformed: 0,
        }
    }

    /// Apply one video datagram, reporting whether anything was painted.
    pub fn apply(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() < VIDEO_HEADER || &bytes[..4] != MAGIC || bytes[4] != Kind::Video as u8 {
            self.malformed += 1;
            return false;
        }
        let seq = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]);
        // `wrapping_sub` rather than `<`, so a sequence that has wrapped past
        // `u32::MAX` is still read as newer rather than as four billion frames
        // of history.
        if self.newest_seq != 0 && seq.wrapping_sub(self.newest_seq) > u32::MAX / 2 {
            self.reordered += 1;
            return false;
        }
        if seq != self.newest_seq {
            self.frames_seen += 1;
            self.newest_seq = seq;
        }

        let tiles = u16::from_le_bytes([bytes[9], bytes[10]]);
        let mut at = VIDEO_HEADER;
        let mut scratch = [0u16; TILE_PIXELS];
        for _ in 0..tiles {
            if at + 4 > bytes.len() {
                self.malformed += 1;
                return false;
            }
            let index = usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
            let length = usize::from(u16::from_le_bytes([bytes[at + 2], bytes[at + 3]]));
            at += 4;
            if index >= TILE_COUNT || at + length > bytes.len() {
                self.malformed += 1;
                return false;
            }
            let Some(used) = unpack_tile(&bytes[at..at + length], &mut scratch) else {
                self.malformed += 1;
                return false;
            };
            debug_assert_eq!(used, length, "a tile's coded length must match its record");
            scatter_tile(&mut self.pixels, index, &scratch);
            self.tiles_applied += 1;
            at += length;
        }
        self.dirty = tiles > 0;
        self.dirty
    }

    /// Take the picture, if anything has been painted since the last call.
    ///
    /// Handed back in the framebuffer's own `0x00RRGGBB` layout, so the front
    /// end's existing conversion and upscaling work on it unchanged.
    pub fn take_screens(&mut self) -> Option<[Vec<u32>; 2]> {
        if !std::mem::take(&mut self.dirty) {
            return None;
        }
        let (top, bottom) = self.pixels.split_at(SCREEN_WIDTH * SCREEN_HEIGHT);
        Some([
            top.iter().map(|p| from_565(*p)).collect(),
            bottom.iter().map(|p| from_565(*p)).collect(),
        ])
    }
}

// -- the wire ------------------------------------------------------------------

/// The remote player's controls, as a whole state.
///
/// Sent every frame whether or not anything changed, and never as a change.
/// Over UDP the datagram carrying "the button came up" is the one that gets
/// lost, and a differential protocol would then hold the button down for good.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Input {
    /// The `melonds::keys` bitmask.
    pub keys: u32,
    /// Where the stylus is, or `None` for lifted.
    pub touch: Option<(u16, u16)>,
    /// Which sample this is, so an overtaking datagram cannot un-press a
    /// button that a newer one pressed.
    pub seq: u32,
}

/// What a session is doing, for the diagnostics pane.
#[derive(Clone, Copy, Default, Debug)]
pub struct RemoteStats {
    pub rtt_ms: f32,
    pub connected: bool,
    /// Frames handed to the encoder, or rebuilt by the decoder.
    pub frames: u64,
    /// Video datagrams sent or received.
    pub video_datagrams: u64,
    /// Video bytes sent or received.
    pub video_bytes: u64,
    /// Audio sample pairs sent or received.
    pub audio_pairs: u64,
    /// Sample pairs dropped to stop the sound drifting behind the picture.
    pub audio_dropped: u64,
    /// Input samples sent or received.
    pub inputs: u64,
    /// Datagrams refused as out of order or malformed.
    pub discarded: u64,
    /// The most recent frame's cost, so the codec's work is visible.
    pub last_frame_tiles: usize,
    pub last_frame_bytes: usize,
}

impl RemoteStats {
    /// The video bit rate implied by the last frame, at the DS's frame rate.
    #[must_use]
    pub fn megabits_per_second(&self) -> f32 {
        self.last_frame_bytes as f32 * 8.0 * 59.83 / 1_000_000.0
    }
}

#[derive(Default)]
struct Counters {
    frames: AtomicU64,
    video_datagrams: AtomicU64,
    video_bytes: AtomicU64,
    audio_pairs: AtomicU64,
    audio_dropped: AtomicU64,
    inputs: AtomicU64,
    discarded: AtomicU64,
    rtt_us: AtomicU64,
    last_frame_tiles: AtomicU64,
    last_frame_bytes: AtomicU64,
}

impl Counters {
    fn snapshot(&self, connected: bool) -> RemoteStats {
        RemoteStats {
            rtt_ms: self.rtt_us.load(Ordering::Relaxed) as f32 / 1000.0,
            connected,
            frames: self.frames.load(Ordering::Relaxed),
            video_datagrams: self.video_datagrams.load(Ordering::Relaxed),
            video_bytes: self.video_bytes.load(Ordering::Relaxed),
            audio_pairs: self.audio_pairs.load(Ordering::Relaxed),
            audio_dropped: self.audio_dropped.load(Ordering::Relaxed),
            inputs: self.inputs.load(Ordering::Relaxed),
            discarded: self.discarded.load(Ordering::Relaxed),
            last_frame_tiles: self.last_frame_tiles.load(Ordering::Relaxed) as usize,
            last_frame_bytes: self.last_frame_bytes.load(Ordering::Relaxed) as usize,
        }
    }

    /// Fold a round-trip sample in, the same 1/8-gain estimator
    /// [`crate::lan`] uses.
    fn observe_rtt(&self, sample: Duration) {
        let sample = sample.as_micros().min(u128::from(u64::MAX)) as u64;
        let previous = self.rtt_us.load(Ordering::Relaxed);
        let smoothed = if previous == 0 { sample } else { (previous * 7 + sample) / 8 };
        self.rtt_us.store(smoothed, Ordering::Relaxed);
    }
}

/// The sender's wall clock in microseconds, for the latency probe.
///
/// Only ever differenced against another reading **on the same machine** — a
/// `Ping` is timed by whoever sent it, from its own echo — so the two clocks do
/// not have to agree.
fn wall_clock_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_micros().min(u128::from(u64::MAX)) as u64)
}

fn header(kind: Kind) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(MAX_DATAGRAM);
    bytes.extend_from_slice(MAGIC);
    bytes.push(kind as u8);
    bytes
}

/// Read a datagram's kind, refusing anything that is not ours.
fn kind_of(bytes: &[u8]) -> Option<Kind> {
    (bytes.len() >= 5 && &bytes[..4] == MAGIC).then(|| Kind::from_wire(bytes[4])).flatten()
}

fn encode_input(input: Input) -> Vec<u8> {
    let mut bytes = header(Kind::Input);
    bytes.extend_from_slice(&input.seq.to_le_bytes());
    bytes.extend_from_slice(&input.keys.to_le_bytes());
    let (x, y, down) = match input.touch {
        Some((x, y)) => (x, y, 1u8),
        None => (0, 0, 0),
    };
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes.push(down);
    bytes
}

fn decode_input(bytes: &[u8]) -> Option<Input> {
    if bytes.len() < 5 + 4 + 4 + 2 + 2 + 1 {
        return None;
    }
    let seq = u32::from_le_bytes(bytes[5..9].try_into().ok()?);
    let keys = u32::from_le_bytes(bytes[9..13].try_into().ok()?);
    let x = u16::from_le_bytes([bytes[13], bytes[14]]);
    let y = u16::from_le_bytes([bytes[15], bytes[16]]);
    let touch = (bytes[17] != 0).then_some((x, y));
    Some(Input { keys, touch, seq })
}

// -- the two ends ----------------------------------------------------------------

/// What both ends share: a socket, a peer, counters, and the threads that keep
/// them fed.
struct Session {
    socket: UdpSocket,
    remote: SocketAddr,
    counters: Counters,
    shutdown: Arc<AtomicBool>,
    /// The newest input the remote player sent — host side only.
    input: Mutex<Input>,
    /// Audio waiting to be played — client side only.
    audio: Mutex<VecDeque<i16>>,
    /// The picture being rebuilt — client side only.
    decoder: Mutex<Decoder>,
    /// How many sample pairs may wait before the oldest are dropped.
    audio_limit: usize,
    /// Sequence for outgoing audio and input.
    out_seq: AtomicU32,
    /// The newest input sequence accepted, so an overtaking datagram cannot
    /// un-press a button a newer one pressed.
    input_seq: AtomicU32,
}

impl Session {
    fn send(&self, bytes: &[u8]) -> bool {
        self.socket.send_to(bytes, self.remote).is_ok()
    }

    /// The receive loop, shared by both ends: every datagram either updates
    /// state or is counted as discarded.
    fn receive_loop(&self) {
        let mut buffer = vec![0u8; MAX_DATAGRAM * 2];
        while !self.shutdown.load(Ordering::Relaxed) {
            let Ok((len, from)) = self.socket.recv_from(&mut buffer) else {
                continue;
            };
            if from != self.remote {
                continue;
            }
            let datagram = &buffer[..len];
            let Some(kind) = kind_of(datagram) else {
                self.counters.discarded.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            match kind {
                Kind::Video => self.receive_video(datagram),
                Kind::Audio => self.receive_audio(datagram),
                Kind::Input => self.receive_input(datagram),
                // Answered here rather than queued: the probe is timing the
                // path, and time spent waiting for another thread to notice is
                // not part of the path.
                Kind::Ping => {
                    let mut echo = header(Kind::Pong);
                    echo.extend_from_slice(&datagram[5..datagram.len().min(13)]);
                    self.send(&echo);
                }
                Kind::Pong => {
                    if datagram.len() >= 13
                        && let Ok(sent) = datagram[5..13].try_into().map(u64::from_le_bytes)
                        && let Some(rtt) = wall_clock_micros().checked_sub(sent)
                    {
                        self.counters.observe_rtt(Duration::from_micros(rtt));
                    }
                }
                Kind::Hello | Kind::Welcome => {}
            }
        }
    }

    fn receive_video(&self, datagram: &[u8]) {
        let mut decoder = self.decoder.lock().unwrap_or_else(|e| e.into_inner());
        let before = decoder.frames_seen;
        if decoder.apply(datagram) {
            self.counters.video_datagrams.fetch_add(1, Ordering::Relaxed);
            self.counters.video_bytes.fetch_add(datagram.len() as u64, Ordering::Relaxed);
            if decoder.frames_seen != before {
                self.counters.frames.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            self.counters.discarded.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn receive_audio(&self, datagram: &[u8]) {
        if datagram.len() < 9 {
            self.counters.discarded.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let samples: Vec<i16> =
            datagram[9..].as_chunks::<2>().0.iter().map(|pair| i16::from_le_bytes(*pair)).collect();
        let mut queue = self.audio.lock().unwrap_or_else(|e| e.into_inner());
        queue.extend(samples.iter().copied());
        // Audio that is queued is audio that is late. Trimming from the front
        // costs one audible moment; not trimming costs a sound track that
        // slides further behind the picture for as long as the session runs.
        let limit = self.audio_limit * 2;
        if queue.len() > limit {
            let excess = queue.len() - limit;
            queue.drain(..excess);
            self.counters.audio_dropped.fetch_add(excess as u64 / 2, Ordering::Relaxed);
        }
        self.counters.audio_pairs.fetch_add(samples.len() as u64 / 2, Ordering::Relaxed);
    }

    fn receive_input(&self, datagram: &[u8]) {
        let Some(input) = decode_input(datagram) else {
            self.counters.discarded.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let newest = self.input_seq.load(Ordering::Relaxed);
        if newest != 0 && input.seq.wrapping_sub(newest) > u32::MAX / 2 {
            self.counters.discarded.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.input_seq.store(input.seq, Ordering::Relaxed);
        *self.input.lock().unwrap_or_else(|e| e.into_inner()) = input;
        self.counters.inputs.fetch_add(1, Ordering::Relaxed);
    }
}

fn start_session(
    socket: UdpSocket,
    remote: SocketAddr,
    tuning: Tuning,
    ping: bool,
) -> io::Result<Arc<Session>> {
    // Short enough that shutdown is prompt, long enough that an idle link is
    // not a spin loop.
    socket.set_read_timeout(Some(Duration::from_millis(50)))?;
    let session = Arc::new(Session {
        socket,
        remote,
        counters: Counters::default(),
        shutdown: Arc::new(AtomicBool::new(false)),
        input: Mutex::new(Input::default()),
        audio: Mutex::new(VecDeque::new()),
        decoder: Mutex::new(Decoder::new()),
        audio_limit: usize::from(tuning.max_audio_lag_ms) * 48,
        out_seq: AtomicU32::new(1),
        input_seq: AtomicU32::new(0),
    });

    let receiver = Arc::clone(&session);
    std::thread::Builder::new()
        .name("melon_egui-remote-rx".to_owned())
        .spawn(move || receiver.receive_loop())
        .map_err(|error| io::Error::other(format!("cannot start remote receiver: {error}")))?;

    if ping {
        let prober = Arc::clone(&session);
        std::thread::Builder::new()
            .name("melon_egui-remote-ping".to_owned())
            .spawn(move || {
                while !prober.shutdown.load(Ordering::Relaxed) {
                    let mut probe = header(Kind::Ping);
                    probe.extend_from_slice(&wall_clock_micros().to_le_bytes());
                    prober.send(&probe);
                    std::thread::sleep(PING_INTERVAL);
                }
            })
            .map_err(|error| io::Error::other(format!("cannot start remote prober: {error}")))?;
    }
    Ok(session)
}

/// The machine both consoles run on: it streams the second one out and takes
/// the remote player's controls back.
pub struct RemoteHost {
    session: Arc<Session>,
    encoder: Mutex<Encoder>,
    tuning: Tuning,
}

impl RemoteHost {
    /// Bind `bind_addr` and wait for one client's `HELLO`.
    ///
    /// Blocks, so the caller runs it off the UI thread.
    ///
    /// # Errors
    /// If the port cannot be bound, or the socket fails while waiting.
    pub fn accept(bind_addr: SocketAddr, mut tuning: Tuning) -> io::Result<Self> {
        tuning.normalize();
        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        let mut buffer = vec![0u8; MAX_DATAGRAM];
        loop {
            match socket.recv_from(&mut buffer) {
                Ok((len, client)) if kind_of(&buffer[..len]) == Some(Kind::Hello) => {
                    let welcome = header(Kind::Welcome);
                    // Three copies: losing the welcome costs the client its
                    // whole connection attempt, and it is five bytes.
                    for _ in 0..3 {
                        socket.send_to(&welcome, client)?;
                    }
                    let session = start_session(socket, client, tuning, true)?;
                    return Ok(Self {
                        session,
                        encoder: Mutex::new(Encoder::new(tuning.refresh_period)),
                        tuning,
                    });
                }
                Ok(_) => continue,
                Err(ref error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Encode one frame and put it on the wire.
    ///
    /// Called from the streamed console's own thread, never from the UI
    /// thread: encoding costs a few milliseconds, and spending them on the
    /// thread that runs the *other* console would reintroduce exactly the frame
    /// rate loss this mode exists to remove.
    pub fn send_frame(&self, top: &[u32], bottom: &[u32]) {
        let mut datagrams = Vec::new();
        let cost = {
            let mut encoder = self.encoder.lock().unwrap_or_else(|e| e.into_inner());
            encoder.encode(top, bottom, &mut datagrams)
        };
        for datagram in &datagrams {
            if self.session.send(datagram) {
                self.session.counters.video_datagrams.fetch_add(1, Ordering::Relaxed);
                self.session
                    .counters
                    .video_bytes
                    .fetch_add(datagram.len() as u64, Ordering::Relaxed);
            }
        }
        let counters = &self.session.counters;
        counters.frames.fetch_add(1, Ordering::Relaxed);
        counters.last_frame_tiles.store(cost.tiles as u64, Ordering::Relaxed);
        counters.last_frame_bytes.store(cost.bytes as u64, Ordering::Relaxed);
    }

    /// Put a run of interleaved stereo samples on the wire.
    ///
    /// `samples` is what the console produced this frame. Anything beyond
    /// [`Tuning::max_audio_lag_ms`] is dropped here rather than sent: a
    /// datagram that arrives too late to play is bandwidth spent making the
    /// sound later still.
    pub fn send_audio(&self, samples: &[i16]) {
        if !self.tuning.audio || samples.is_empty() {
            return;
        }
        let limit = self.session.audio_limit * 2;
        let samples = if samples.len() > limit {
            let dropped = (samples.len() - limit) / 2;
            self.session.counters.audio_dropped.fetch_add(dropped as u64, Ordering::Relaxed);
            &samples[samples.len() - limit..]
        } else {
            samples
        };
        for chunk in samples.chunks(AUDIO_CHUNK_PAIRS * 2) {
            let seq = self.session.out_seq.fetch_add(1, Ordering::Relaxed);
            let mut datagram = header(Kind::Audio);
            datagram.extend_from_slice(&seq.to_le_bytes());
            for sample in chunk {
                datagram.extend_from_slice(&sample.to_le_bytes());
            }
            if self.session.send(&datagram) {
                self.session
                    .counters
                    .audio_pairs
                    .fetch_add(chunk.len() as u64 / 2, Ordering::Relaxed);
            }
        }
    }

    /// The remote player's current controls.
    #[must_use]
    pub fn input(&self) -> Input {
        *self.session.input.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The address this end is bound to.
    ///
    /// # Errors
    /// If the socket cannot report it.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.session.socket.local_addr()
    }

    /// The client that connected.
    #[must_use]
    pub fn remote_addr(&self) -> SocketAddr {
        self.session.remote
    }

    /// What the session is doing, for the diagnostics pane.
    #[must_use]
    pub fn stats(&self) -> RemoteStats {
        self.session.counters.snapshot(true)
    }
}

/// The machine that only displays: no cart, no save, no emulation.
pub struct RemoteClient {
    session: Arc<Session>,
}

impl RemoteClient {
    /// Bind `bind_addr`, announce to `host_addr`, and wait for its welcome.
    ///
    /// Retries, because on a VPN the first datagram after the tunnel comes up is
    /// the one most likely to be dropped.
    ///
    /// # Errors
    /// If the port cannot be bound, or no welcome arrives.
    pub fn connect(
        bind_addr: SocketAddr,
        host_addr: SocketAddr,
        mut tuning: Tuning,
    ) -> io::Result<Self> {
        tuning.normalize();
        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_read_timeout(Some(Duration::from_secs(1)))?;
        let hello = header(Kind::Hello);
        let mut buffer = vec![0u8; MAX_DATAGRAM];
        for _ in 0..10 {
            socket.send_to(&hello, host_addr)?;
            match socket.recv_from(&mut buffer) {
                Ok((len, from))
                    if from == host_addr && kind_of(&buffer[..len]) == Some(Kind::Welcome) =>
                {
                    return Ok(Self { session: start_session(socket, host_addr, tuning, false)? });
                }
                Ok(_) => continue,
                Err(ref error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("no answer from {host_addr} after 10 attempts"),
        ))
    }

    /// Send this repaint's controls.
    ///
    /// Called every repaint whatever the player is doing — see [`Input`].
    pub fn send_input(&self, keys: u32, touch: Option<(u16, u16)>) {
        let seq = self.session.out_seq.fetch_add(1, Ordering::Relaxed);
        if self.session.send(&encode_input(Input { keys, touch, seq })) {
            self.session.counters.inputs.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The newest picture, if anything has been painted since the last call.
    pub fn take_screens(&self) -> Option<[Vec<u32>; 2]> {
        self.session.decoder.lock().unwrap_or_else(|e| e.into_inner()).take_screens()
    }

    /// Take everything waiting to be played.
    pub fn take_audio(&self) -> Vec<i16> {
        let mut queue = self.session.audio.lock().unwrap_or_else(|e| e.into_inner());
        queue.drain(..).collect()
    }

    /// The address this end is bound to.
    ///
    /// # Errors
    /// If the socket cannot report it.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.session.socket.local_addr()
    }

    /// The host this end is watching.
    #[must_use]
    pub fn remote_addr(&self) -> SocketAddr {
        self.session.remote
    }

    /// What the session is doing, for the diagnostics pane.
    #[must_use]
    pub fn stats(&self) -> RemoteStats {
        self.session.counters.snapshot(true)
    }
}

/// Winding the receive and probe threads up is the same at both ends, and has
/// to happen however the session ends.
macro_rules! impl_drop {
    ($type:ty) => {
        impl Drop for $type {
            fn drop(&mut self) {
                self.session.shutdown.store(true, Ordering::Relaxed);
            }
        }
    };
}

impl_drop!(RemoteHost);
impl_drop!(RemoteClient);

#[cfg(test)]
mod tests {
    use std::{sync::Mutex, time::Duration};

    use super::{
        Decoder, Encoder, FRAME_PIXELS, Input, RemoteClient, RemoteHost, SCREEN_HEIGHT,
        SCREEN_WIDTH, TILE, TILE_COUNT, TILE_PIXELS, Tuning, decode_input, encode_input, from_565,
        pack_tile, to_565, unpack_tile,
    };

    /// One screen's worth of framebuffer pixels, in the core's `0x00RRGGBB`.
    fn screen(fill: impl Fn(usize, usize) -> u32) -> Vec<u32> {
        (0..SCREEN_WIDTH * SCREEN_HEIGHT)
            .map(|at| fill(at % SCREEN_WIDTH, at / SCREEN_WIDTH))
            .collect()
    }

    /// The picture a decoder would show, for comparison against what was fed in.
    /// Every reference frame is put through 565 first, since that is the codec's
    /// declared precision and not a defect.
    fn quantised(pixels: &[u32]) -> Vec<u32> {
        pixels.iter().map(|p| from_565(to_565(*p))).collect()
    }

    #[test]
    fn colour_survives_the_round_trip_within_565() {
        for pixel in [0x00_00_00_00, 0x00_FF_FF_FF, 0x00_FF_00_00, 0x00_00_FF_00, 0x00_00_00_FF] {
            let back = from_565(to_565(pixel));
            for shift in [16, 8, 0] {
                let (before, after) = ((pixel >> shift) & 0xFF, (back >> shift) & 0xFF);
                assert!(
                    before.abs_diff(after) <= 4,
                    "channel at {shift} moved from {before} to {after}"
                );
            }
        }
        // The two that must be exact, or every bright area is tinted.
        assert_eq!(from_565(to_565(0x00_FF_FF_FF)), 0x00_FF_FF_FF);
        assert_eq!(from_565(to_565(0)), 0);
    }

    #[test]
    fn a_tile_round_trips_whatever_is_in_it() {
        for tile in [
            // Flat: the case the run coder is for.
            [0x1234u16; TILE_PIXELS],
            // Alternating: the case that must not be *worse* than raw.
            std::array::from_fn(|i| if i % 2 == 0 { 0xFFFF } else { 0 }),
            // Pseudo-random: the worst case.
            std::array::from_fn(|i| (i as u16).wrapping_mul(40_503)),
        ] {
            let mut packed = Vec::new();
            pack_tile(&tile, &mut packed);
            let mut back = [0u16; TILE_PIXELS];
            let used = unpack_tile(&packed, &mut back).expect("a well-formed tile");
            assert_eq!(used, packed.len());
            assert_eq!(back, tile);
            // Two bytes over raw is the coder's declared worst case.
            assert!(
                packed.len() <= TILE_PIXELS * 2 + 2,
                "a tile coded to {} bytes, over the {} ceiling",
                packed.len(),
                TILE_PIXELS * 2 + 2
            );
        }
    }

    #[test]
    fn a_flat_tile_costs_almost_nothing() {
        let mut packed = Vec::new();
        pack_tile(&[0x07E0; TILE_PIXELS], &mut packed);
        assert!(packed.len() <= 8, "a flat tile coded to {} bytes", packed.len());
    }

    #[test]
    fn a_malformed_tile_is_refused_rather_than_trusted() {
        let mut packed = Vec::new();
        pack_tile(&[0x1234; TILE_PIXELS], &mut packed);
        let mut back = [0u16; TILE_PIXELS];
        // Truncated.
        assert!(unpack_tile(&packed[..packed.len() - 1], &mut back).is_none());
        // Empty.
        assert!(unpack_tile(&[], &mut back).is_none());
        // A literal token promising more pixels than the tile holds.
        assert!(unpack_tile(&[0x7F, 0, 0], &mut back).is_none());
    }

    /// The whole codec, end to end: what goes in must come out, to 565.
    #[test]
    fn a_first_frame_arrives_complete() {
        let top = screen(|x, y| ((x as u32) << 16) | ((y as u32) << 8) | 0x40);
        let bottom = screen(|x, y| ((y as u32) << 16) | 0x2000 | (x as u32));

        let mut encoder = Encoder::new(8);
        let mut datagrams = Vec::new();
        let cost = encoder.encode(&top, &bottom, &mut datagrams);
        assert_eq!(cost.tiles, TILE_COUNT, "the first frame must send every tile");

        let mut decoder = Decoder::new();
        for datagram in &datagrams {
            assert!(decoder.apply(datagram));
        }
        let [got_top, got_bottom] = decoder.take_screens().expect("a painted frame");
        assert_eq!(got_top, quantised(&top));
        assert_eq!(got_bottom, quantised(&bottom));
    }

    /// The point of the delta: a still picture costs only its refresh slice.
    #[test]
    fn a_still_picture_costs_only_the_rolling_refresh() {
        let top = screen(|x, y| ((x as u32 / 8) << 16) | ((y as u32 / 8) << 8));
        let bottom = screen(|_, _| 0);

        let mut encoder = Encoder::new(8);
        let mut datagrams = Vec::new();
        encoder.encode(&top, &bottom, &mut datagrams);
        let repeat = encoder.encode(&top, &bottom, &mut datagrams);

        // One eighth of the tiles, give or take the rounding of 384/8.
        assert!(
            repeat.tiles <= TILE_COUNT / 8 + 1,
            "an unchanged frame sent {} tiles, more than one refresh slice",
            repeat.tiles
        );
    }

    /// Every datagram must stand alone — that is what removes the need for
    /// acknowledgements, keyframes and reassembly.
    #[test]
    fn each_datagram_is_applicable_on_its_own() {
        let top = screen(|x, y| ((x as u32) << 16) | ((y as u32) << 8));
        let bottom = screen(|x, y| ((x as u32 ^ y as u32) << 8) | 0x11);

        let mut encoder = Encoder::new(8);
        let mut datagrams = Vec::new();
        encoder.encode(&top, &bottom, &mut datagrams);
        assert!(datagrams.len() > 4, "the test needs a frame that spans several datagrams");

        // Applied out of order, and with one thrown away, every surviving
        // datagram still paints its own tiles.
        let mut decoder = Decoder::new();
        let mut shuffled: Vec<&Vec<u8>> = datagrams.iter().collect();
        shuffled.reverse();
        for datagram in shuffled.iter().skip(1) {
            assert!(decoder.apply(datagram));
        }
        assert!(decoder.take_screens().is_some());
    }

    /// The loss-recovery claim, measured: whatever is dropped, the rolling
    /// refresh repaints it within one period and the picture converges exactly.
    #[test]
    fn a_lossy_link_converges_within_one_refresh_period() {
        const PERIOD: u8 = 8;
        let top = screen(|x, y| ((x as u32) << 16) | ((y as u32) << 8) | 0x33);
        let bottom = screen(|x, y| ((y as u32) << 16) | ((x as u32) << 8) | 0x77);

        let mut encoder = Encoder::new(PERIOD);
        let mut decoder = Decoder::new();
        let mut datagrams = Vec::new();
        // A third of every frame's datagrams are thrown away — far worse than
        // any usable link — for long enough that a period has certainly passed.
        let mut noise: u32 = 0x1234_5678;
        for frame in 0..(u32::from(PERIOD) * 3) {
            encoder.encode(&top, &bottom, &mut datagrams);
            for datagram in &datagrams {
                noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                // The last period is delivered intact, which is what "within
                // one refresh period" means.
                if frame < u32::from(PERIOD) * 2 && (noise >> 16).is_multiple_of(3) {
                    continue;
                }
                decoder.apply(datagram);
            }
        }
        let [got_top, got_bottom] = decoder.take_screens().expect("a painted frame");
        assert_eq!(got_top, quantised(&top), "the top screen did not converge");
        assert_eq!(got_bottom, quantised(&bottom), "the bottom screen did not converge");
    }

    /// A number rather than an assertion: what the codec actually costs on
    /// DS-like content. Printed by `cargo test -- --nocapture`, so the
    /// bandwidth claim is measured rather than asserted.
    #[test]
    fn the_codec_reports_what_it_costs() {
        /// Roughly what a DS game looks like: a flat background, a large
        /// scrolling area, and a small sprite moving quickly.
        fn game_frame(frame: usize) -> (Vec<u32>, Vec<u32>) {
            let scroll = frame * 2;
            let top = screen(|x, y| {
                let stripe = ((x + scroll) / 32).is_multiple_of(2);
                let bg = if stripe { 0x0020_4060 } else { 0x0018_3850 };
                let sprite = frame * 3 % (SCREEN_WIDTH - 32);
                if x >= sprite && x < sprite + 32 && (80..112).contains(&y) {
                    0x00E0_C040
                } else {
                    bg
                }
            });
            // A menu: mostly still, which is what most of a DS screen is.
            let bottom = screen(|x, y| if y % 48 < 4 || x < 8 { 0x0080_8080 } else { 0x0010_1018 });
            (top, bottom)
        }

        for period in [4u8, 8, 16] {
            let mut encoder = Encoder::new(period);
            let mut datagrams = Vec::new();
            let mut total_bytes = 0usize;
            let mut total_tiles = 0usize;
            const FRAMES: usize = 120;
            // The first frame is a full send by definition, so it is measured
            // separately rather than smeared over the average.
            let (top, bottom) = game_frame(0);
            let first = encoder.encode(&top, &bottom, &mut datagrams);
            for frame in 1..FRAMES {
                let (top, bottom) = game_frame(frame);
                let cost = encoder.encode(&top, &bottom, &mut datagrams);
                total_bytes += cost.bytes;
                total_tiles += cost.tiles;
            }
            let per_frame = total_bytes as f64 / (FRAMES - 1) as f64;
            println!(
                "refresh_period {period:2}: first frame {:6} B / {:3} tiles | \
                 steady {:7.0} B/frame, {:5.1} tiles, {:5.2} Mbit/s at 59.83 fps",
                first.bytes,
                first.tiles,
                per_frame,
                total_tiles as f64 / (FRAMES - 1) as f64,
                per_frame * 8.0 * 59.83 / 1_000_000.0,
            );
            // A DS screen pair is 384 KiB raw and 192 KiB at 565; anything near
            // that means the delta is not working at all.
            assert!(
                per_frame < (FRAME_PIXELS * 2) as f64 / 2.0,
                "refresh_period {period} averaged {per_frame} bytes a frame, no better than raw"
            );
        }
    }

    /// A datagram from somewhere else must be refused, not indexed with.
    #[test]
    fn a_foreign_datagram_is_refused() {
        let mut decoder = Decoder::new();
        assert!(!decoder.apply(b"not ours"));
        assert!(!decoder.apply(&[]));
        // Right magic, wrong kind.
        assert!(!decoder.apply(b"MRD1\x04rubbish"));
    }

    /// Input is a whole state and must survive the wire exactly, lifted stylus
    /// included — a touch that decoded as `Some((0, 0))` would drag the stylus
    /// to the corner of the screen every time the player lifted it.
    #[test]
    fn input_round_trips_including_a_lifted_stylus() {
        for input in [
            Input { keys: 0, touch: None, seq: 1 },
            Input { keys: 0x3FF, touch: Some((128, 96)), seq: 2 },
            Input { keys: 1, touch: Some((0, 0)), seq: u32::MAX },
        ] {
            let back = decode_input(&encode_input(input)).expect("a well-formed input");
            assert_eq!(back, input);
        }
        assert!(decode_input(b"MRD1\x04short").is_none());
    }

    #[test]
    fn tuning_clamps_a_hand_edited_file() {
        let mut tuning = Tuning { refresh_period: 0, audio: true, max_audio_lag_ms: 5, port: 0 };
        tuning.normalize();
        assert_eq!(tuning.refresh_period, 1);
        assert_eq!(tuning.max_audio_lag_ms, 20);
        assert_eq!(tuning.port, Tuning::default().port);
    }

    // -- the session, end to end ---------------------------------------------

    /// A `RemoteHost` and a `RemoteClient` really talking, over a relay that
    /// adds a VPN's delay, jitter and loss.
    ///
    /// This is the claim that matters: a session survives a link that LAN mode
    /// could not, because nothing here waits for a round trip. What is asserted
    /// is that the picture arrives and converges, that the controls get back,
    /// and that the round trip is measured — the same three things a player
    /// would check.
    #[test]
    fn a_session_survives_a_lossy_delayed_link() {
        use std::{
            sync::atomic::{AtomicBool, Ordering},
            time::Instant,
        };

        // -- a relay that delays, jitters and drops -------------------------
        let host_probe = std::net::UdpSocket::bind("127.0.0.1:0").expect("a host port");
        let host_addr = host_probe.local_addr().expect("the host address");
        drop(host_probe);

        let relay = std::net::UdpSocket::bind("127.0.0.1:0").expect("a relay port");
        let relay_addr = relay.local_addr().expect("the relay address");
        relay.set_read_timeout(Some(Duration::from_millis(20))).expect("a relay timeout");
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        type Queued = (Instant, std::net::SocketAddr, Vec<u8>);
        let queue: std::sync::Arc<Mutex<Vec<Queued>>> = Default::default();
        let relay = std::sync::Arc::new(relay);
        for reading in [true, false] {
            let (relay, queue, stop) = (
                std::sync::Arc::clone(&relay),
                std::sync::Arc::clone(&queue),
                std::sync::Arc::clone(&stop),
            );
            std::thread::spawn(move || {
                let mut client: Option<std::net::SocketAddr> = None;
                let mut buffer = vec![0u8; 4096];
                let mut noise: u32 = 0x9E37_79B9;
                /// A bad link by any measure: far worse than the 16.9 ms VPN
                /// that LAN mode was failing on.
                const LOSS_PERCENT: u32 = 8;
                while !stop.load(Ordering::Relaxed) {
                    if reading {
                        let Ok((len, from)) = relay.recv_from(&mut buffer) else { continue };
                        noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        // 8% loss, which is a bad link by any measure.
                        if (noise >> 16) % 100 < LOSS_PERCENT {
                            continue;
                        }
                        let to = if from == host_addr {
                            match client {
                                Some(client) => client,
                                None => continue,
                            }
                        } else {
                            client = Some(from);
                            host_addr
                        };
                        // 20 ms each way, ±4 ms: a 40 ms round trip, past
                        // anything LAN mode could have played over.
                        let extra = Duration::from_micros(u64::from((noise >> 8) % 4000));
                        let at = Instant::now() + Duration::from_millis(20) + extra;
                        queue.lock().unwrap().push((at, to, buffer[..len].to_vec()));
                    } else {
                        let now = Instant::now();
                        let due: Vec<Queued> = {
                            let mut queue = queue.lock().unwrap();
                            let (due, rest) =
                                queue.drain(..).partition::<Vec<_>, _>(|(at, ..)| *at <= now);
                            *queue = rest;
                            due
                        };
                        for (_, to, bytes) in due {
                            let _ = relay.send_to(&bytes, to);
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            });
        }

        // -- the two ends ---------------------------------------------------
        let tuning = Tuning { refresh_period: 8, ..Tuning::default() };
        let accepting =
            std::thread::spawn(move || RemoteHost::accept(host_addr, tuning).expect("the host"));
        let client = RemoteClient::connect("127.0.0.1:0".parse().unwrap(), relay_addr, tuning)
            .expect("the client connects");
        let host = accepting.join().expect("the accept thread");

        // -- a console's worth of frames ------------------------------------
        let top = screen(|x, y| ((x as u32) << 16) | ((y as u32) << 8) | 0x55);
        let bottom = screen(|x, y| ((y as u32) << 16) | ((x as u32) << 8) | 0x99);
        let frame_time = Duration::from_secs_f64(1.0 / 59.83);
        let mut slot = Instant::now();
        // Three refresh periods, plus room for the probe to land.
        for frame in 0..90 {
            host.send_frame(&top, &bottom);
            if frame % 4 == 0 {
                // What one frame of 48 kHz stereo looks like.
                host.send_audio(&vec![0i16; 800 * 2]);
            }
            // The client's side of a repaint: controls out, picture in.
            client.send_input(0x0F, Some((100, 50)));
            let _ = client.take_screens();
            slot += frame_time;
            let now = Instant::now();
            if slot > now {
                std::thread::sleep(slot - now);
            } else {
                slot = now;
            }
        }
        // Long enough for the last frames and a ping to complete the path.
        std::thread::sleep(Duration::from_millis(600));
        let _ = client.take_screens();
        host.send_frame(&top, &bottom);
        std::thread::sleep(Duration::from_millis(200));

        let host_stats = host.stats();
        let client_stats = client.stats();
        println!("host:   {host_stats:#?}");
        println!("client: {client_stats:#?}");

        // The picture got there, and converged exactly despite 8% loss.
        let [got_top, got_bottom] = client.take_screens().expect("a picture");
        assert_eq!(got_top, quantised(&top), "the top screen did not converge");
        assert_eq!(got_bottom, quantised(&bottom), "the bottom screen did not converge");

        // The controls got back, as a whole state.
        let input = host.input();
        assert_eq!(input.keys, 0x0F);
        assert_eq!(input.touch, Some((100, 50)));
        assert!(host_stats.inputs > 50, "only {} input samples arrived", host_stats.inputs);

        // The sound got there.
        assert!(client_stats.audio_pairs > 0, "no audio reached the client");

        // And the link was measured: ~40 ms through the relay.
        assert!(
            host_stats.rtt_ms > 25.0 && host_stats.rtt_ms < 120.0,
            "the probe measured {} ms across a ~40 ms relay",
            host_stats.rtt_ms
        );

        stop.store(true, Ordering::Relaxed);
    }

    /// The tile grid must address every pixel exactly once, or the rolling
    /// refresh would leave a stripe of the screen permanently stale.
    #[test]
    fn the_tile_grid_covers_the_frame_exactly() {
        let mut seen = vec![0u8; FRAME_PIXELS];
        for index in 0..TILE_COUNT {
            let origin = super::tile_origin(index);
            for row in 0..TILE {
                for column in 0..TILE {
                    seen[origin + row * SCREEN_WIDTH + column] += 1;
                }
            }
        }
        assert!(seen.iter().all(|count| *count == 1), "the tile grid overlaps or leaves gaps");
    }
}
