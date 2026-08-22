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
//! player sees `round trip + a frame or two`. That is worse than sitting at the
//! host machine and far better than 29.6 fps with two rounds in five failing.
//!
//! Because every console is on the host, **the host holds all of it**: saves,
//! savestates, cheats, instance directories. A client owns nothing and is asked
//! for nothing.
//!
//! # What is in here
//!
//! | module | what it does |
//! |--------|--------------|
//! | [`colour`] | 8888 ↔ 565, the only place channel order is decided |
//! | [`tile`] | the 16×16 grid and its run/literal coder |
//! | [`encoder`] | frame → datagrams, including which frames to skip |
//! | [`decoder`] | datagrams → frame, in any order, with any of them missing |
//! | [`audio`] | 48 kHz → transport rate, so sound is not most of the bandwidth |
//! | [`wire`] | the datagram layouts |
//! | [`stats`] | what a session is doing, for the pane |
//! | [`session`] | the socket, the threads, and the shared state |
//! | [`host`] / [`client`] | the two ends |
//! | [`tuning`] | the knobs, and their bounds |
//!
//! # The video codec, and why it needs no acknowledgements
//!
//! Each screen is cut into 16×16 tiles ([`tile::TILE`]). A tile is encoded
//! whole, in RGB565, with a PackBits run/literal coder — DS art is mostly flat
//! colour, which that suits. A frame sends the tiles that **changed**, plus the
//! tiles whose turn it is in a rolling refresh (see
//! [`Tuning::refresh_period`]).
//!
//! Whole tiles are packed into MTU-sized datagrams, so **every datagram is
//! independently applicable**: a client that receives one applies its tiles and
//! is that much more correct, whatever else was lost. There is no reassembly,
//! no acknowledgement, no keyframe and no request-for-keyframe — a lost
//! datagram leaves a few tiles stale for at most one refresh period, and then
//! they are painted again by the rolling refresh.
//!
//! # Keeping it light enough to feel immediate
//!
//! Two things dominate the bandwidth, and both are cut rather than merely
//! tuned:
//!
//! * **Sound was over a third of it.** 48 kHz stereo `i16` is 1.5 Mbit/s on its
//!   own. It travels at [`Tuning::audio_rate`] instead — halved by default — and
//!   the client resamples back up on the way to the sound card, so what is lost
//!   is bandwidth rather than audible quality. See [`audio`].
//! * **Not every frame is worth sending.** The console runs at 59.83 fps
//!   whatever the link does; the *picture* is sent at [`Tuning::max_video_fps`]
//!   and falls further if the measured bit rate exceeds
//!   [`Tuning::max_bitrate_kbps`]. Skipping is nearly free here — the delta
//!   simply accumulates into the next frame that is sent — and, crucially, it
//!   does not add latency: a datagram is put on the wire the moment it is
//!   encoded, so a *skipped* frame delays nothing. Only smoothness is traded,
//!   never immediacy. See [`encoder::Pacer`].

pub mod audio;
pub mod client;
pub mod colour;
pub mod decoder;
pub mod encoder;
pub mod host;
pub mod session;
pub mod stats;
pub mod tile;
pub mod tuning;
pub mod wire;

#[cfg(test)]
mod tests;

pub use client::RemoteClient;
pub use host::RemoteHost;
pub use stats::RemoteStats;
pub use tuning::Tuning;
#[cfg(test)]
pub use {decoder::Decoder, encoder::Encoder};

/// One DS screen, as `melonds::SCREEN_WIDTH` / `SCREEN_HEIGHT`.
///
/// Repeated here rather than imported so this module builds without the
/// emulator core, which is what lets the codec be tested on any machine.
pub const SCREEN_WIDTH: usize = 256;
/// See [`SCREEN_WIDTH`].
pub const SCREEN_HEIGHT: usize = 192;
/// Pixels in a frame — both screens.
pub const FRAME_PIXELS: usize = SCREEN_WIDTH * SCREEN_HEIGHT * 2;

/// How large a datagram may grow.
///
/// Under a typical VPN's reduced MTU (WireGuard defaults to 1420) less its own
/// headers, so that a video datagram is never IP-fragmented — losing one
/// fragment would lose every tile in it, which is exactly the coupling this
/// codec exists to avoid.
pub const MAX_DATAGRAM: usize = 1200;

/// The rate the console's SPU hands samples over at, as
/// [`crate::audio::SPU_SAMPLE_RATE`] explains.
///
/// Repeated for the same reason as [`SCREEN_WIDTH`]: this module has to build
/// without the emulator core.
pub const CONSOLE_SAMPLE_RATE: u32 = 48_000;

/// The DS's video frame rate, `33_513_982 / 560_190` Hz.
pub const NATIVE_FPS: f64 = 59.826_098;
