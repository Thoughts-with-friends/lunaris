//! The multiplayer transport abstraction shared by every MP backend.
//!
//! Port of melonDS's `src/net/MPInterface.h` / `MPInterface.cpp` (vendored
//! for reference at `docs/design/melonds/net/`). A backend moves DS
//! wireless MP frames between emulator *instances*; every call therefore
//! carries an instance index, exactly as in melonDS, because one backend
//! object is shared by all instances running in the process.
//!
//! Differences from the C++ original, all forced by dropping raw pointers:
//!
//! * `u8* data, int len` pairs become `&[u8]` / `&mut [u8]`.
//! * `int` returns that overload "byte count" with "-1 = host gone" become
//!   [`MpRecvResult`].
//! * The packed `u32 Type` word survives as [`MpFrameType`] rather than
//!   being split into separate fields, because melonDS packs the
//!   association ID into its upper half (`SendReply` sends `2 | (aid<<16)`)
//!   and [`MpInterface::recv_replies`] reads it back out with `Type >> 16`.
//! * `MPInterface::Set` / `Get` (a process-global `unique_ptr`) has no
//!   equivalent: the frontend owns the selected backend instead. See
//!   `lunaris_net`'s `mp_interface` module.

use std::time::Duration;

/// Maximum number of emulator instances a backend can serve. melonDS uses a
/// `u16` connected-instance bitmask throughout, which fixes this at 16.
pub const MAX_INSTANCES: usize = 16;

/// `'NIFI'` — tags every MP packet header so a FIFO overrun is detected
/// rather than silently misparsed.
pub const MP_PACKET_MAGIC: u32 = 0x4946_494E;

/// Default blocking receive budget, matching melonDS's `RecvTimeout = 25`.
pub const DEFAULT_RECV_TIMEOUT: Duration = Duration::from_millis(25);

/// Which MP backend is selected.
///
/// Mirrors melonDS's `MPInterfaceType`. `Netplay` is listed for parity with
/// the original enum but has no implementation in lunaris.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MpInterfaceType {
    /// No multiplayer; every operation is a no-op ([`DummyMp`]).
    #[default]
    Dummy,
    /// Instances sharing one process, exchanging frames through in-memory
    /// FIFOs ([`super::local::LocalMp`]).
    Local,
    /// Instances on separate machines, exchanging frames over sockets.
    /// Implemented by the frontend (`lunaris_net`), not by `nds-core`.
    Lan,
    /// Rollback/lockstep netplay. Not implemented.
    Netplay,
}

/// The four MP frame categories carried in the low half of
/// [`MpFrameType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpFrameCategory {
    /// Regular data/beacon/auth/association traffic.
    Regular = 0,
    /// Host multiplayer command frame.
    Cmd = 1,
    /// Client multiplayer reply frame; carries an association ID.
    Reply = 2,
    /// Host multiplayer acknowledgement frame.
    Ack = 3,
}

/// melonDS's packed `MPPacketHeader::Type` word: category in the low 16
/// bits, association ID in the high 16 bits (reply frames only).
///
/// Kept packed rather than split into two fields so that the encoding
/// round-trips bit-for-bit with melonDS's wire header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MpFrameType(u32);

impl MpFrameType {
    /// A regular data frame.
    pub const REGULAR: Self = Self(MpFrameCategory::Regular as u32);
    /// A host command frame.
    pub const CMD: Self = Self(MpFrameCategory::Cmd as u32);
    /// A host acknowledgement frame.
    pub const ACK: Self = Self(MpFrameCategory::Ack as u32);

    /// A client reply frame from association ID `aid`.
    #[must_use]
    pub const fn reply(aid: u16) -> Self {
        Self(MpFrameCategory::Reply as u32 | ((aid as u32) << 16))
    }

    /// Reconstructs the type word from its raw wire encoding.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The raw wire encoding.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The frame category, or `None` if the low half holds a value melonDS
    /// never emits (only reachable from a corrupt FIFO read).
    #[must_use]
    pub const fn category(self) -> Option<MpFrameCategory> {
        match self.0 & 0xFFFF {
            0 => Some(MpFrameCategory::Regular),
            1 => Some(MpFrameCategory::Cmd),
            2 => Some(MpFrameCategory::Reply),
            3 => Some(MpFrameCategory::Ack),
            _ => None,
        }
    }

    /// The association ID packed into the upper half. Meaningful only for
    /// [`MpFrameCategory::Reply`]; zero otherwise.
    #[must_use]
    pub const fn aid(self) -> u16 {
        (self.0 >> 16) as u16
    }
}

/// Header prefixed to every frame placed in an MP FIFO.
///
/// Byte-for-byte equivalent to melonDS's `MPPacketHeader`: five
/// little-endian fields, with the `u64` timestamp 8-byte aligned, giving a
/// 24-byte encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MpPacketHeader {
    /// Always [`MP_PACKET_MAGIC`] on a well-formed header.
    pub magic: u32,
    /// Index of the instance that sent this frame.
    pub sender_id: u32,
    /// Packed category + association ID.
    pub frame_type: MpFrameType,
    /// Payload length in bytes, excluding this header.
    pub length: u32,
    /// Sender's emulated microsecond clock at transmission time.
    pub timestamp: u64,
}

impl MpPacketHeader {
    /// Encoded size in bytes (`sizeof(MPPacketHeader)` in melonDS).
    pub const ENCODED_LEN: usize = 24;

    /// Serializes to the 24-byte little-endian wire form.
    #[must_use]
    pub fn to_bytes(self) -> [u8; Self::ENCODED_LEN] {
        let mut out = [0u8; Self::ENCODED_LEN];
        out[0..4].copy_from_slice(&self.magic.to_le_bytes());
        out[4..8].copy_from_slice(&self.sender_id.to_le_bytes());
        out[8..12].copy_from_slice(&self.frame_type.bits().to_le_bytes());
        out[12..16].copy_from_slice(&self.length.to_le_bytes());
        out[16..24].copy_from_slice(&self.timestamp.to_le_bytes());
        out
    }

    /// Deserializes from the 24-byte little-endian wire form. The magic
    /// field is *not* validated here; callers check it so they can run
    /// melonDS's FIFO-overflow recovery path.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; Self::ENCODED_LEN]) -> Self {
        // Every slice below is a fixed, in-bounds sub-range of a
        // fixed-size array, so the `try_into` conversions cannot fail.
        let word = |lo: usize| -> u32 {
            u32::from_le_bytes([bytes[lo], bytes[lo + 1], bytes[lo + 2], bytes[lo + 3]])
        };
        MpPacketHeader {
            magic: word(0),
            sender_id: word(4),
            frame_type: MpFrameType::from_bits(word(8)),
            length: word(12),
            timestamp: u64::from_le_bytes([
                bytes[16], bytes[17], bytes[18], bytes[19], bytes[20], bytes[21], bytes[22],
                bytes[23],
            ]),
        }
    }
}

/// Outcome of a receive call on an [`MpInterface`].
///
/// Replaces melonDS's overloaded `int` return, where `0` meant "nothing"
/// and `-1` (from `RecvHostPacket` only) meant "the host went away".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpRecvResult {
    /// Nothing available within the receive budget.
    None,
    /// The instance that last sent a CMD frame is no longer connected, so
    /// the MP session must end. Only [`MpInterface::recv_host_packet`]
    /// reports this.
    HostGone,
    /// A frame was copied into the caller's buffer.
    Frame {
        /// Bytes written. May be smaller than the sender's frame if the
        /// caller's buffer was shorter.
        len: usize,
        /// Packed category + association ID from the frame header.
        ///
        /// melonDS discards this (its `RecvPacket` returns only a length),
        /// but it costs nothing to surface and lets
        /// [`super::bridge::MpInterfaceTransport`] fill in
        /// [`crate::nds::MpFrameKind`] without re-parsing the frame body.
        frame_type: MpFrameType,
        /// Sender's emulated microsecond clock at transmission time.
        timestamp: u64,
    },
}

/// A multiplayer backend: moves MP frames between emulator instances.
///
/// Port of melonDS's abstract `MPInterface` class. Implementations must be
/// [`Send`] because the emulator core runs on whichever thread owns
/// [`crate::nds::NDS`].
///
/// # Errors
/// Deliberately infallible, like the original: transient failures surface
/// as [`MpRecvResult::None`] or [`MpRecvResult::HostGone`], because the
/// Wi-Fi hardware has no error-recovery path — it just keeps polling.
pub trait MpInterface: Send {
    /// Called once per video frame, for backends that need to pump an
    /// event loop. Socket-free backends leave this empty.
    fn process(&mut self) {}

    /// Registers `inst` as ready to send and receive frames.
    fn begin(&mut self, inst: u8);

    /// Deregisters `inst`.
    fn end(&mut self, inst: u8);

    /// Broadcasts a regular data frame. Returns the number of bytes
    /// accepted (`0` if the frame was rejected as oversized).
    fn send_packet(&mut self, inst: u8, data: &[u8], timestamp: u64) -> usize;

    /// Non-blocking poll of the regular receive FIFO.
    fn recv_packet(&mut self, inst: u8, data: &mut [u8]) -> MpRecvResult;

    /// Broadcasts a host command frame, opening a reply window.
    fn send_cmd(&mut self, inst: u8, data: &[u8], timestamp: u64) -> usize;

    /// Sends a client reply frame tagged with association ID `aid`.
    fn send_reply(&mut self, inst: u8, data: &[u8], timestamp: u64, aid: u16) -> usize;

    /// Broadcasts a host acknowledgement frame.
    fn send_ack(&mut self, inst: u8, data: &[u8], timestamp: u64) -> usize;

    /// Blocking receive (bounded by [`MpInterface::recv_timeout`]) used by
    /// clients waiting on the host's next frame. Reports
    /// [`MpRecvResult::HostGone`] once the last known host disconnects.
    fn recv_host_packet(&mut self, inst: u8, data: &mut [u8]) -> MpRecvResult;

    /// Host-only: collects client reply frames matching `timestamp` from
    /// the clients named in `aid_mask`, returning the bitmask of
    /// association IDs that answered.
    ///
    /// `data` is laid out as 1 KiB slots indexed by `aid - 1`, matching
    /// `crate::hw::wifi`'s `mp_client_replies` buffer.
    fn recv_replies(&mut self, inst: u8, data: &mut [u8], timestamp: u64, aid_mask: u16) -> u16;

    /// Current blocking-receive budget.
    fn recv_timeout(&self) -> Duration;

    /// Overrides the blocking-receive budget.
    fn set_recv_timeout(&mut self, timeout: Duration);
}

/// The no-op backend selected when multiplayer is off.
///
/// Port of melonDS's file-local `DummyMP`, which is what
/// `MPInterface::Current` holds until `MPInterface::Set` installs
/// something real.
#[derive(Debug, Clone, Copy)]
pub struct DummyMp {
    recv_timeout: Duration,
}

impl Default for DummyMp {
    fn default() -> Self {
        DummyMp { recv_timeout: DEFAULT_RECV_TIMEOUT }
    }
}

impl DummyMp {
    /// Creates the no-op backend.
    #[must_use]
    pub const fn new() -> Self {
        DummyMp { recv_timeout: DEFAULT_RECV_TIMEOUT }
    }
}

impl MpInterface for DummyMp {
    fn begin(&mut self, _inst: u8) {}
    fn end(&mut self, _inst: u8) {}

    fn send_packet(&mut self, _inst: u8, _data: &[u8], _timestamp: u64) -> usize {
        0
    }

    fn recv_packet(&mut self, _inst: u8, _data: &mut [u8]) -> MpRecvResult {
        MpRecvResult::None
    }

    fn send_cmd(&mut self, _inst: u8, _data: &[u8], _timestamp: u64) -> usize {
        0
    }

    fn send_reply(&mut self, _inst: u8, _data: &[u8], _timestamp: u64, _aid: u16) -> usize {
        0
    }

    fn send_ack(&mut self, _inst: u8, _data: &[u8], _timestamp: u64) -> usize {
        0
    }

    fn recv_host_packet(&mut self, _inst: u8, _data: &mut [u8]) -> MpRecvResult {
        MpRecvResult::None
    }

    fn recv_replies(
        &mut self,
        _inst: u8,
        _data: &mut [u8],
        _timestamp: u64,
        _aid_mask: u16,
    ) -> u16 {
        0
    }

    fn recv_timeout(&self) -> Duration {
        self.recv_timeout
    }

    fn set_recv_timeout(&mut self, timeout: Duration) {
        self.recv_timeout = timeout;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_type_round_trips_category_and_aid() {
        for aid in 0..16u16 {
            let ty = MpFrameType::reply(aid);
            assert_eq!(ty.category(), Some(MpFrameCategory::Reply));
            assert_eq!(ty.aid(), aid);
            // melonDS sends exactly `2 | (aid << 16)`.
            assert_eq!(ty.bits(), 2 | (u32::from(aid) << 16));
            assert_eq!(MpFrameType::from_bits(ty.bits()), ty);
        }
    }

    #[test]
    fn non_reply_types_carry_no_aid() {
        for ty in [MpFrameType::REGULAR, MpFrameType::CMD, MpFrameType::ACK] {
            assert_eq!(ty.aid(), 0);
        }
        assert_eq!(MpFrameType::REGULAR.category(), Some(MpFrameCategory::Regular));
        assert_eq!(MpFrameType::CMD.category(), Some(MpFrameCategory::Cmd));
        assert_eq!(MpFrameType::ACK.category(), Some(MpFrameCategory::Ack));
    }

    #[test]
    fn packet_header_round_trips() {
        let header = MpPacketHeader {
            magic: MP_PACKET_MAGIC,
            sender_id: 3,
            frame_type: MpFrameType::reply(7),
            length: 0x948,
            timestamp: 0x0123_4567_89AB_CDEF,
        };
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), MpPacketHeader::ENCODED_LEN);
        assert_eq!(MpPacketHeader::from_bytes(&bytes), header);
    }
}
