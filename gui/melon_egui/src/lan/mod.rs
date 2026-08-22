//! A UDP transport for melonDS local wireless that survives a VPN.
//!
//! # Why this exists beside `melonds::lan`
//!
//! `melonds-rs` ships a LAN transport of its own, and on a real LAN it works.
//! Over a VPN it does not, and the two symptoms the user sees have one cause
//! between them:
//!
//! * `melonds::lan::RECEIVE_TIMEOUT` is a fixed **25 ms**, and
//!   `mp_recv_replies` blocks for it once per emulated frame. On a link whose
//!   round trip exceeds 25 ms the guest's reply *cannot* arrive in time, so the
//!   host collects nothing and the game reports a communication error — and
//!   the 25 ms it spent waiting is subtracted from every frame, which is the
//!   frame rate collapse.
//! * `melonds::lan::STALE_REPLY_US` is a fixed 32 000 emulated microseconds, so
//!   a reply that *does* arrive, merely late, is then thrown away as stale.
//!
//! Those are `const`s inside a git dependency pinned by revision, so they
//! cannot be tuned from here. `melonds::Host` is public, however, and
//! `Emu::boot_lan` already takes any `Box<dyn melonds::Host>` — so this module
//! replaces the transport rather than patching it.
//!
//! # What it does differently
//!
//! Four things, all of which are only available to an emulator: real hardware
//! has none of these choices.
//!
//! 1. **The wait is measured, not guessed.** `PING`/`PONG` frames ride the same
//!    socket, and the round-trip estimate they produce sets both the reply wait
//!    and the staleness window ([`Link::budget`]). A 150 ms VPN gets a 150 ms
//!    budget; a LAN keeps a short one and stays responsive.
//! 2. **Replies are sent more than once.** A dropped datagram on a
//!    round-synchronous protocol is a lost round, and a lost round is a
//!    communication error. Sending each reply [`Tuning::reply_copies`] times
//!    turns single-packet loss into no event at all; duplicates are discarded
//!    by sequence number.
//! 3. **Ordinary packets are batched.** Beacons and the association handshake
//!    are not round-synchronous, so several may share one datagram
//!    ([`Coalescer`]). This is the "let some pile up before sending" the link
//!    actually permits — see *Why CMD/reply rounds cannot be batched* below.
//! 4. **The emulated clock follows the link.** [`LinkPace`] reports the frame
//!    rate the link can sustain, and the front end paces the console to it
//!    instead of to 59.83 Hz. The console then runs *slightly slow and
//!    connected* rather than *at full speed and disconnected*, and the front
//!    end stops accumulating a frame debt it can only discharge by flooding the
//!    peer with a burst of rounds.
//!
//! # Why CMD/reply rounds cannot be batched
//!
//! It is worth being plain about the limit, because "buffer up N frames and
//! send them together" is the obvious thing to ask for and it does not work
//! here. A DS wireless round is synchronous *within one emulated frame*: the
//! host sends CMD, every addressed client answers, and the host's ACK — and the
//! game logic behind it — depends on those answers before the frame ends
//! (GBATEK, "DS Wifi ... Multiplay"). Holding round N back to send it with
//! round N+1 means round N's answers arrive after the host needed them, which
//! is the same communication error by a different route. Batching is therefore
//! applied to `Generic` frames only, and latency is absorbed by (1), (2) and
//! (4) instead.

mod batching;
mod endpoints;
mod measure;
mod peer;
mod queue;
mod tuning;
mod wire;

#[cfg(test)]
mod harness;
#[cfg(test)]
mod tests;

use std::{
    collections::VecDeque,
    io,
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) use batching::Coalescer;
pub use endpoints::{LanGuest, LanHost};
pub(crate) use measure::Measurements;
pub use measure::{LinkPace, LinkStats};
pub(crate) use peer::Peer;
pub(crate) use queue::Queue;
pub use tuning::Tuning;
pub(crate) use wire::{decode, encode_into, stamp_sequence};

/// Identifies this transport's datagrams. Deliberately not `melonds::lan`'s
/// `MLAN`: the layouts differ, and a mismatched pair should fail to handshake
/// rather than exchange frames it will misread.
const MAGIC: &[u8; 4] = b"MLN2";

/// `MAGIC` + kind + aid + timestamp + sequence + length.
const HEADER_LEN: usize = 4 + 1 + 2 + 8 + 4 + 2;

/// The largest wireless frame the DS moves, melonDS's `kMaxFrameSize`.
const MAX_PAYLOAD: usize = 0x948;

/// How large a coalesced datagram may grow before it is flushed.
///
/// Chosen to stay under a typical VPN's reduced MTU (WireGuard's default 1420,
/// less its own headers) so that batching does not simply move the loss into IP
/// fragmentation, where losing one fragment loses the whole datagram.
const MAX_DATAGRAM: usize = 1200;

/// How many datagram sequence numbers are remembered for duplicate rejection.
///
/// Only has to cover the reordering window, which is a handful of frames even
/// on a bad link; the memory is trivial either way.
const SEEN_WINDOW: usize = 512;

/// How many frames a queue holds before the *oldest* is dropped.
///
/// `melonds::lan` uses 32, which a jitter burst overruns — and it drops by
/// arrival order, discarding frames that are still live. This is large enough
/// that eviction is a genuine overload rather than ordinary jitter, and
/// [`Queue::push`] drops by age instead.
const QUEUE_CAPACITY: usize = 256;

/// How long a queued frame may sit before it is certainly answering something
/// that is over.
///
/// A DS wireless round lasts one emulated frame — 16.7 ms — so anything this
/// old is stale by two orders of magnitude, whatever the link is doing.
const STALE_FRAME_AGE: Duration = Duration::from_secs(1);

/// How often a `PING` is sent, in wall time.
const PING_INTERVAL: Duration = Duration::from_millis(250);

/// What a datagram's frames are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// Beacons, association, deauth: everything outside an MP round.
    Packet = 0,
    /// The host's "reply to me now", which opens a round.
    Cmd = 1,
    /// A client's answer, tagged with the AID the host gave it.
    Reply = 2,
    /// The host's "I heard you", which closes a round.
    Ack = 3,
    /// Handshake: a guest announcing itself.
    Hello = 4,
    /// Handshake: the host accepting.
    Welcome = 5,
    /// Latency probe. Its `timestamp` is the sender's wall clock in
    /// microseconds, echoed verbatim in the `Pong`.
    Ping = 6,
    /// Latency probe echo.
    Pong = 7,
}

impl Kind {
    const fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Packet),
            1 => Some(Self::Cmd),
            2 => Some(Self::Reply),
            3 => Some(Self::Ack),
            4 => Some(Self::Hello),
            5 => Some(Self::Welcome),
            6 => Some(Self::Ping),
            7 => Some(Self::Pong),
            _ => None,
        }
    }

    /// Whether a frame of this kind belongs to a round and must go out at once.
    const fn is_urgent(self) -> bool {
        matches!(self, Self::Cmd | Self::Reply | Self::Ack | Self::Ping | Self::Pong)
    }
}

/// One wireless frame in flight.
pub(crate) struct Frame {
    pub(crate) kind: Kind,
    pub(crate) aid: u16,
    /// The sender's emulated wifi clock, in microseconds — except on
    /// `Ping`/`Pong`, where it is the sender's wall clock.
    pub(crate) timestamp: u64,
    pub(crate) payload: Vec<u8>,
}
