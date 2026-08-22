//! What travels on the airwaves: a frame, its kind, and the log line it
//! leaves behind.

/// What kind of frame a packet is, which decides the queue it lands in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Beacons, the association handshake, deauth — everything before and
    /// outside an MP round.
    Generic,
    /// The host's "reply to me now", which starts a round.
    Cmd,
    /// A client's answer, tagged with the AID the host gave it.
    Reply(u16),
    /// The host's "I heard you", which closes a round.
    Ack,
}

impl Kind {
    /// The label used in the diagnostics window.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Generic => "packet",
            Self::Cmd => "CMD",
            Self::Reply(_) => "reply",
            Self::Ack => "ACK",
        }
    }
}

/// One frame in flight.
#[derive(Clone)]
pub(crate) struct Packet {
    pub(crate) sender: usize,
    pub(crate) kind: Kind,
    pub(crate) timestamp: u64,
    pub(crate) data: Vec<u8>,
}

/// A line in the traffic log.
#[derive(Clone)]
pub struct Event {
    pub sender: usize,
    pub kind: Kind,
    pub timestamp: u64,
    pub len: usize,
}

/// The largest frame the wifi hardware moves, melonDS's `kMaxFrameSize`.
/// `SendPacketGeneric` refuses anything bigger and warns rather than truncating
/// it into the queue, and so does this.
pub(crate) const MAX_FRAME_SIZE: usize = 0x948;

/// A reply slot is 1024 bytes, and `recv_replies` is handed one buffer holding
/// all of them (melonDS `kMaxFrameSize` reasoning; the wrapper sizes its buffer
/// at 16 KiB for exactly this).
pub(crate) const REPLY_SLOT: usize = 1024;

/// How many log lines are kept.
pub(crate) const LOG_LIMIT: usize = 400;
