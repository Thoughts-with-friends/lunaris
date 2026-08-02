//! Local multiplayer: MP frames exchanged between emulator instances
//! sharing one process, through two in-memory ring FIFOs.
//!
//! Port of melonDS's `src/net/LocalMP.cpp` / `LocalMP.h`. The layout is
//! identical: a packet FIFO carrying regular/CMD/ack frames, a reply FIFO
//! carrying client replies, one shared write cursor per FIFO, one read
//! cursor per instance, and a pool of 32 semaphores (0-15 = "instance *i*
//! has a frame to read", 16-31 = "instance *i* has a reply to read").
//!
//! # Ownership model
//! melonDS has one `LocalMP` object addressed by all instances through the
//! `inst` parameter; sharing is implicit because the object is reached via
//! a process-global `MPInterface::Current`. Here the shared state is an
//! explicit [`LocalMpHub`] behind an [`Arc`], and [`LocalMp`] is a handle
//! onto it — so several instances, on several threads, can each hold their
//! own [`MpInterface`] while talking through one hub. The `inst`
//! parameter is preserved, so the call flow matches the original exactly.
//!
//! # Deliberate deviations
//! Every one of these is a case where melonDS relies on a raw pointer
//! staying in bounds by convention:
//!
//! * A header whose `length` exceeds [`MAX_FRAME_SIZE`] is treated as FIFO
//!   corruption (resynchronise and drop) instead of being trusted.
//! * `recv_replies` writes reply *aid* at `data[(aid - 1) * 1024]`, as in
//!   melonDS, but rejects `aid == 0` (which would underflow) and `aid > 15`,
//!   and clips a slot that would run past the caller's buffer.
//! * Receive calls copy at most `data.len()` bytes into the caller's
//!   buffer, while still consuming the whole frame from the FIFO, so a
//!   short buffer desynchronises nothing.

use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use super::semaphore::Semaphore;
use crate::hw::net::mp_interface::{
    DEFAULT_RECV_TIMEOUT, MAX_INSTANCES, MP_PACKET_MAGIC, MpFrameCategory, MpFrameType,
    MpInterface, MpPacketHeader, MpRecvResult,
};

/// Size of the regular-frame ring FIFO, in bytes.
pub const PACKET_QUEUE_SIZE: usize = 0x1_0000;
/// Size of the reply ring FIFO, in bytes.
pub const REPLY_QUEUE_SIZE: usize = 0x1_0000;
/// Largest MP frame accepted for transmission, in bytes.
pub const MAX_FRAME_SIZE: usize = 0x948;

/// Per-client slot size inside the caller's `recv_replies` buffer.
const REPLY_SLOT_SIZE: usize = 1024;
/// Index of the first reply semaphore in the pool.
const REPLY_SEM_BASE: usize = 16;
/// Total semaphores: 16 regular + 16 reply.
const SEM_POOL_SIZE: usize = 32;

/// Shared bookkeeping for the two FIFOs.
///
/// Port of melonDS's `MPStatusData`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MpStatusData {
    /// Bitmask of instances currently ready to send and receive.
    pub connected_bitmask: u16,
    /// Shared write cursor into the packet FIFO.
    pub packet_write_offset: u32,
    /// Shared write cursor into the reply FIFO.
    pub reply_write_offset: u32,
    /// Instance that sent the most recent CMD frame.
    pub mp_host_inst: u16,
    /// Bitmask of instances that replied to that CMD frame in time.
    pub mp_reply_bitmask: u16,
}

/// Selects one of the two FIFOs. melonDS passes `int fifo` with `0` =
/// packets and `1` = replies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fifo {
    Packet,
    Reply,
}

/// The mutex-guarded half of [`LocalMpHub`].
struct Queues {
    status: MpStatusData,
    /// Boxed rather than inline: 64 KiB each would otherwise be a large
    /// stack array during construction.
    packet_queue: Box<[u8]>,
    reply_queue: Box<[u8]>,
    packet_read_offset: [u32; MAX_INSTANCES],
    reply_read_offset: [u32; MAX_INSTANCES],
    /// Instance that sent the last CMD frame seen by *any* receiver, or
    /// `None` before the first one. melonDS keeps this as `LastHostID`
    /// with a `-1` sentinel, read outside the queue lock; holding the lock
    /// costs nothing here and removes the race.
    last_host_id: Option<u8>,
}

impl Queues {
    fn new() -> Self {
        Queues {
            status: MpStatusData::default(),
            packet_queue: vec![0u8; PACKET_QUEUE_SIZE].into_boxed_slice(),
            reply_queue: vec![0u8; REPLY_QUEUE_SIZE].into_boxed_slice(),
            packet_read_offset: [0; MAX_INSTANCES],
            reply_read_offset: [0; MAX_INSTANCES],
            last_host_id: None,
        }
    }

    /// Reads `buf.len()` bytes from `fifo` at instance `inst`'s read cursor,
    /// wrapping around the end of the ring and advancing the cursor.
    ///
    /// Mirrors `LocalMP::FIFORead`, including its `>=` wrap test: when the
    /// read ends exactly at the ring's end, the cursor must land on `0`
    /// rather than on `datalen`, which is what keeps the
    /// `cursor < capacity` invariant true.
    fn fifo_read(&mut self, inst: usize, fifo: Fifo, buf: &mut [u8]) {
        let (data, offset) = match fifo {
            Fifo::Packet => (&self.packet_queue, &mut self.packet_read_offset[inst]),
            Fifo::Reply => (&self.reply_queue, &mut self.reply_read_offset[inst]),
        };
        let datalen = data.len();
        let start = *offset as usize;
        let len = buf.len();
        debug_assert!(start < datalen, "FIFO read cursor left the ring");
        debug_assert!(len <= datalen, "FIFO read larger than the ring itself");

        if start + len >= datalen {
            let part1 = datalen - start;
            buf[..part1].copy_from_slice(&data[start..]);
            buf[part1..].copy_from_slice(&data[..len - part1]);
            *offset = (len - part1) as u32;
        } else {
            buf.copy_from_slice(&data[start..start + len]);
            *offset += len as u32;
        }
    }

    /// Writes `buf` into `fifo` at the shared write cursor, wrapping around
    /// the end of the ring and advancing the cursor.
    ///
    /// Mirrors `LocalMP::FIFOWrite`. Like the original, this does not check
    /// whether the FIFO is full — an overrun is detected downstream by the
    /// reader, whose next header read fails the magic check and triggers a
    /// resynchronise.
    fn fifo_write(&mut self, fifo: Fifo, buf: &[u8]) {
        let (data, offset) = match fifo {
            Fifo::Packet => (&mut self.packet_queue, &mut self.status.packet_write_offset),
            Fifo::Reply => (&mut self.reply_queue, &mut self.status.reply_write_offset),
        };
        let datalen = data.len();
        let start = *offset as usize;
        let len = buf.len();
        debug_assert!(start < datalen, "FIFO write cursor left the ring");
        debug_assert!(len <= datalen, "FIFO write larger than the ring itself");

        if start + len >= datalen {
            let part1 = datalen - start;
            data[start..].copy_from_slice(&buf[..part1]);
            data[..len - part1].copy_from_slice(&buf[part1..]);
            *offset = (len - part1) as u32;
        } else {
            data[start..start + len].copy_from_slice(buf);
            *offset += len as u32;
        }
    }

    /// Advances a read cursor past a frame body without copying it, used
    /// for the "skip this packet" paths.
    const fn skip(&mut self, inst: usize, fifo: Fifo, len: u32) {
        let (capacity, offset) = match fifo {
            Fifo::Packet => (PACKET_QUEUE_SIZE as u32, &mut self.packet_read_offset[inst]),
            Fifo::Reply => (REPLY_QUEUE_SIZE as u32, &mut self.reply_read_offset[inst]),
        };
        *offset = (*offset + len) % capacity;
    }

    /// Force-resynchronises a read cursor onto the write cursor after a
    /// detected FIFO overrun.
    const fn resync(&mut self, inst: usize, fifo: Fifo) {
        match fifo {
            Fifo::Packet => self.packet_read_offset[inst] = self.status.packet_write_offset,
            Fifo::Reply => self.reply_read_offset[inst] = self.status.reply_write_offset,
        }
    }
}

/// The state every local-MP participant shares: both FIFOs, the connected
/// bitmask, and the semaphore pool.
///
/// Wrap in an [`Arc`] and hand a [`LocalMp`] handle to each instance.
pub struct LocalMpHub {
    queues: Mutex<Queues>,
    /// `[0..16)` signal a readable regular frame for that instance;
    /// `[16..32)` signal a readable reply.
    sem_pool: [Semaphore; SEM_POOL_SIZE],
}

impl Default for LocalMpHub {
    fn default() -> Self {
        LocalMpHub::new()
    }
}

impl LocalMpHub {
    /// Creates an empty hub with both FIFOs zeroed and no instance
    /// connected.
    #[must_use]
    pub fn new() -> Self {
        LocalMpHub {
            queues: Mutex::new(Queues::new()),
            sem_pool: std::array::from_fn(|_| Semaphore::new()),
        }
    }

    /// A copy of the current shared status word. Diagnostics only.
    #[must_use]
    pub fn status(&self) -> MpStatusData {
        self.lock().status
    }

    fn lock(&self) -> MutexGuard<'_, Queues> {
        // A panic while holding this lock cannot leave the FIFOs in a
        // state a reader can't recover from: the magic check plus
        // `resync` already handle arbitrary garbage. Recovering beats
        // poisoning the whole MP session.
        self.queues.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Registers `inst`: its read cursors jump to the current write
    /// cursors (so it never sees frames sent before it joined) and its
    /// semaphores are cleared.
    ///
    /// Port of `LocalMP::Begin`.
    pub fn begin(&self, inst: u8) {
        let Some(index) = instance_index(inst) else { return };
        let mut queues = self.lock();
        queues.packet_read_offset[index] = queues.status.packet_write_offset;
        queues.reply_read_offset[index] = queues.status.reply_write_offset;
        queues.status.connected_bitmask |= 1 << index;
        drop(queues);
        self.sem_pool[index].reset();
        self.sem_pool[REPLY_SEM_BASE + index].reset();
    }

    /// Deregisters `inst`. Port of `LocalMP::End`.
    pub fn end(&self, inst: u8) {
        let Some(index) = instance_index(inst) else { return };
        self.lock().status.connected_bitmask &= !(1 << index);
    }

    /// Queues a frame and wakes whoever needs to read it.
    ///
    /// Port of `LocalMP::SendPacketGeneric`. Reply frames go to the reply
    /// FIFO and wake only the current host; everything else goes to the
    /// packet FIFO and wakes every connected instance (senders filter
    /// their own frames out on receive).
    pub fn send_packet_generic(
        &self,
        inst: u8,
        frame_type: MpFrameType,
        packet: &[u8],
        timestamp: u64,
    ) -> usize {
        let Some(index) = instance_index(inst) else { return 0 };
        let len = packet.len();
        if len > MAX_FRAME_SIZE {
            log::warn!("wifi: attempting to send frame too big (len={len} max={MAX_FRAME_SIZE})");
            return 0;
        }

        let category = frame_type.category();
        let fifo =
            if category == Some(MpFrameCategory::Reply) { Fifo::Reply } else { Fifo::Packet };
        let header = MpPacketHeader {
            magic: MP_PACKET_MAGIC,
            sender_id: index as u32,
            frame_type,
            length: len as u32,
            timestamp,
        };

        let mut queues = self.lock();
        let connected = queues.status.connected_bitmask;

        queues.fifo_write(fifo, &header.to_bytes());
        if len != 0 {
            queues.fifo_write(fifo, packet);
        }

        match category {
            Some(MpFrameCategory::Cmd) => {
                // Opening a reply window: this instance becomes the host,
                // and its view of the reply FIFO restarts from here.
                queues.status.mp_host_inst = index as u16;
                queues.status.mp_reply_bitmask = 0;
                queues.reply_read_offset[index] = queues.status.reply_write_offset;
                self.sem_pool[REPLY_SEM_BASE + index].reset();
            }
            Some(MpFrameCategory::Reply) => {
                queues.status.mp_reply_bitmask |= 1 << index;
            }
            _ => {}
        }
        // Read inside the lock: melonDS reads `MPStatus.MPHostinst` after
        // unlocking, which can pick up a different host than the one this
        // reply was addressed to.
        let host_inst = queues.status.mp_host_inst;
        drop(queues);

        if category == Some(MpFrameCategory::Reply) {
            if let Some(host) = instance_index(host_inst as u8) {
                self.sem_pool[REPLY_SEM_BASE + host].post();
            }
        } else {
            for i in 0..MAX_INSTANCES {
                if connected & (1 << i) != 0 {
                    self.sem_pool[i].post();
                }
            }
        }

        len
    }

    /// Pops the next frame addressed to `inst` from the packet FIFO,
    /// waiting up to `timeout` (pass [`Duration::ZERO`] for melonDS's
    /// non-blocking `block = false`).
    ///
    /// Port of `LocalMP::RecvPacketGeneric`.
    pub fn recv_packet_generic(
        &self,
        inst: u8,
        packet: &mut [u8],
        timeout: Duration,
    ) -> MpRecvResult {
        let Some(index) = instance_index(inst) else { return MpRecvResult::None };

        loop {
            if !self.sem_pool[index].try_wait(timeout) {
                return MpRecvResult::None;
            }

            let mut queues = self.lock();
            let Some(header) = read_header(&mut queues, index, Fifo::Packet) else {
                log::warn!("PACKET FIFO OVERFLOW");
                queues.resync(index, Fifo::Packet);
                drop(queues);
                self.sem_pool[index].reset();
                return MpRecvResult::None;
            };

            if header.sender_id == index as u32 {
                // Our own broadcast came back around; drop it.
                queues.skip(index, Fifo::Packet, header.length);
                continue;
            }

            let mut body = [0u8; MAX_FRAME_SIZE];
            let len = header.length as usize;
            if len != 0 {
                queues.fifo_read(index, Fifo::Packet, &mut body[..len]);
                if header.frame_type.category() == Some(MpFrameCategory::Cmd) {
                    queues.last_host_id = u8::try_from(header.sender_id).ok();
                }
            }
            drop(queues);

            let copied = len.min(packet.len());
            packet[..copied].copy_from_slice(&body[..copied]);
            return MpRecvResult::Frame {
                len: copied,
                frame_type: header.frame_type,
                timestamp: header.timestamp,
            };
        }
    }

    /// Blocking receive for a client waiting on its host, reporting
    /// [`MpRecvResult::HostGone`] once the last known host has left.
    ///
    /// Port of `LocalMP::RecvHostPacket`.
    pub fn recv_host_packet(&self, inst: u8, packet: &mut [u8], timeout: Duration) -> MpRecvResult {
        let queues = self.lock();
        if let Some(host) = queues.last_host_id
            && queues.status.connected_bitmask & (1 << host) == 0
        {
            return MpRecvResult::HostGone;
        }
        drop(queues);

        self.recv_packet_generic(inst, packet, timeout)
    }

    /// Host-only: drains the reply FIFO for replies to the CMD frame sent
    /// at `timestamp`, returning the bitmask of association IDs that
    /// answered.
    ///
    /// Port of `LocalMP::RecvReplies`. `data` is addressed as 1 KiB slots
    /// indexed by `aid - 1`.
    pub fn recv_replies(
        &self,
        inst: u8,
        data: &mut [u8],
        timestamp: u64,
        aid_mask: u16,
        timeout: Duration,
    ) -> u16 {
        let Some(index) = instance_index(inst) else { return 0 };

        let mut answered = 0u16;
        let mut seen_mask = 1u16 << index;
        let connected = self.lock().status.connected_bitmask;

        // Every connected instance is us: there is nobody to reply.
        if seen_mask & connected == connected {
            return 0;
        }

        loop {
            if !self.sem_pool[REPLY_SEM_BASE + index].try_wait(timeout) {
                // No more replies within the budget; report what we got.
                return answered;
            }

            let mut queues = self.lock();
            let Some(header) = read_header(&mut queues, index, Fifo::Reply) else {
                log::warn!("REPLY FIFO OVERFLOW");
                queues.resync(index, Fifo::Reply);
                drop(queues);
                self.sem_pool[REPLY_SEM_BASE + index].reset();
                return 0;
            };

            // `timestamp - 32` is deliberately wrapping: melonDS relies on
            // the unsigned underflow at the very start of a session, where
            // it makes the staleness test vacuously true.
            let stale = header.timestamp < timestamp.wrapping_sub(32);
            if header.sender_id == index as u32 || stale {
                queues.skip(index, Fifo::Reply, header.length);
                continue;
            }

            let len = header.length as usize;
            if len != 0 {
                let mut body = [0u8; MAX_FRAME_SIZE];
                queues.fifo_read(index, Fifo::Reply, &mut body[..len]);

                // melonDS indexes `packets[(aid - 1) * 1024]` unchecked;
                // aid 0 would underflow and aid > 15 would run off the
                // 15 KiB buffer, so both are dropped here instead.
                let aid = header.frame_type.aid();
                if (1..MAX_INSTANCES as u16).contains(&aid) {
                    let slot = (aid as usize - 1) * REPLY_SLOT_SIZE;
                    let end = (slot + len).min(data.len());
                    if end > slot {
                        data[slot..end].copy_from_slice(&body[..end - slot]);
                    }
                    answered |= 1 << aid;
                }
            }

            seen_mask |= 1 << (header.sender_id & 0xF);
            drop(queues);

            if seen_mask & connected == connected || answered & aid_mask == aid_mask {
                // Every client has answered.
                return answered;
            }
        }
    }
}

/// Reads and validates one frame header from `fifo`, returning `None` if
/// the magic word is wrong or the advertised length is impossible — both
/// of which mean the FIFO was overrun by a writer.
fn read_header(queues: &mut Queues, index: usize, fifo: Fifo) -> Option<MpPacketHeader> {
    let mut bytes = [0u8; MpPacketHeader::ENCODED_LEN];
    queues.fifo_read(index, fifo, &mut bytes);
    let header = MpPacketHeader::from_bytes(&bytes);
    if header.magic != MP_PACKET_MAGIC || header.length as usize > MAX_FRAME_SIZE {
        return None;
    }
    Some(header)
}

/// Validates an instance index against the 16-entry connected bitmask.
const fn instance_index(inst: u8) -> Option<usize> {
    if (inst as usize) < MAX_INSTANCES { Some(inst as usize) } else { None }
}

/// One instance's handle onto a [`LocalMpHub`].
///
/// Port of melonDS's `LocalMP` class. Construct with [`LocalMp::new`] for a
/// hub of its own, or [`LocalMp::from_hub`] to join an existing one.
pub struct LocalMp {
    hub: Arc<LocalMpHub>,
    recv_timeout: Duration,
}

impl Default for LocalMp {
    fn default() -> Self {
        LocalMp::new()
    }
}

impl LocalMp {
    /// Creates a handle onto a brand-new, empty hub.
    #[must_use]
    pub fn new() -> Self {
        LocalMp::from_hub(Arc::new(LocalMpHub::new()))
    }

    /// Creates a second handle onto an existing hub, so another instance
    /// can join the same local session.
    #[must_use]
    pub const fn from_hub(hub: Arc<LocalMpHub>) -> Self {
        LocalMp { hub, recv_timeout: DEFAULT_RECV_TIMEOUT }
    }

    /// The hub this handle talks through, for creating further handles.
    #[must_use]
    pub const fn hub(&self) -> &Arc<LocalMpHub> {
        &self.hub
    }
}

impl MpInterface for LocalMp {
    fn begin(&mut self, inst: u8) {
        self.hub.begin(inst);
    }

    fn end(&mut self, inst: u8) {
        self.hub.end(inst);
    }

    fn send_packet(&mut self, inst: u8, data: &[u8], timestamp: u64) -> usize {
        self.hub.send_packet_generic(inst, MpFrameType::REGULAR, data, timestamp)
    }

    fn recv_packet(&mut self, inst: u8, data: &mut [u8]) -> MpRecvResult {
        self.hub.recv_packet_generic(inst, data, Duration::ZERO)
    }

    fn send_cmd(&mut self, inst: u8, data: &[u8], timestamp: u64) -> usize {
        self.hub.send_packet_generic(inst, MpFrameType::CMD, data, timestamp)
    }

    fn send_reply(&mut self, inst: u8, data: &[u8], timestamp: u64, aid: u16) -> usize {
        self.hub.send_packet_generic(inst, MpFrameType::reply(aid), data, timestamp)
    }

    fn send_ack(&mut self, inst: u8, data: &[u8], timestamp: u64) -> usize {
        self.hub.send_packet_generic(inst, MpFrameType::ACK, data, timestamp)
    }

    fn recv_host_packet(&mut self, inst: u8, data: &mut [u8]) -> MpRecvResult {
        self.hub.recv_host_packet(inst, data, self.recv_timeout)
    }

    fn recv_replies(&mut self, inst: u8, data: &mut [u8], timestamp: u64, aid_mask: u16) -> u16 {
        self.hub.recv_replies(inst, data, timestamp, aid_mask, self.recv_timeout)
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

    /// Builds two handles onto one hub, with instances 0 and 1 connected.
    fn connected_pair() -> (LocalMp, LocalMp) {
        let mut host = LocalMp::new();
        let mut client = LocalMp::from_hub(Arc::clone(host.hub()));
        host.begin(0);
        client.begin(1);
        (host, client)
    }

    #[test]
    fn regular_frame_reaches_the_other_instance() {
        let (mut host, mut client) = connected_pair();
        assert_eq!(host.send_packet(0, &[1, 2, 3], 100), 3);

        let mut buf = [0u8; 64];
        let recv = client.recv_packet(1, &mut buf);
        assert_eq!(
            recv,
            MpRecvResult::Frame { len: 3, frame_type: MpFrameType::REGULAR, timestamp: 100 }
        );
        assert_eq!(&buf[..3], &[1, 2, 3]);
    }

    #[test]
    fn sender_never_receives_its_own_frame() {
        let (mut host, _client) = connected_pair();
        host.send_packet(0, &[9], 10);
        let mut buf = [0u8; 16];
        assert_eq!(host.recv_packet(0, &mut buf), MpRecvResult::None);
    }

    #[test]
    fn cmd_reply_exchange_reports_the_answering_aid() {
        let (mut host, mut client) = connected_pair();
        host.send_cmd(0, &[0xAA; 8], 1_000);

        // The client sees the CMD frame, then answers as aid 1.
        let mut buf = [0u8; 64];
        let recv = client.recv_host_packet(1, &mut buf);
        assert_eq!(
            recv,
            MpRecvResult::Frame { len: 8, frame_type: MpFrameType::CMD, timestamp: 1_000 }
        );
        client.send_reply(1, &[0x55; 4], 1_000, 1);

        let mut replies = vec![0u8; 15 * REPLY_SLOT_SIZE];
        let answered = host.recv_replies(0, &mut replies, 1_000, 1 << 1);
        assert_eq!(answered, 1 << 1);
        // aid 1 lands in slot 0.
        assert_eq!(&replies[..4], &[0x55; 4]);
    }

    #[test]
    fn stale_replies_are_skipped() {
        let (mut host, mut client) = connected_pair();
        host.send_cmd(0, &[0], 10_000);
        let mut buf = [0u8; 64];
        client.recv_host_packet(1, &mut buf);
        // Older than the `timestamp - 32` staleness threshold.
        client.send_reply(1, &[1, 2], 9_000, 1);

        let mut replies = vec![0u8; 15 * REPLY_SLOT_SIZE];
        host.set_recv_timeout(Duration::from_millis(5));
        assert_eq!(host.recv_replies(0, &mut replies, 10_000, 1 << 1), 0);
    }

    #[test]
    fn early_session_replies_are_all_treated_as_stale() {
        // With `timestamp < 32`, `timestamp - 32` underflows to a huge
        // value and melonDS's staleness test becomes vacuously true, so
        // every reply is dropped. Reproduced deliberately: `wrapping_sub`
        // keeps that behaviour while stopping the debug-build panic Rust
        // would otherwise raise.
        let (mut host, mut client) = connected_pair();
        host.send_cmd(0, &[0], 4);
        let mut buf = [0u8; 64];
        client.recv_host_packet(1, &mut buf);
        client.send_reply(1, &[7], 4, 1);

        let mut replies = vec![0u8; 15 * REPLY_SLOT_SIZE];
        host.set_recv_timeout(Duration::from_millis(5));
        assert_eq!(host.recv_replies(0, &mut replies, 4, 1 << 1), 0);
    }

    #[test]
    fn replies_are_accepted_once_the_clock_is_past_the_stale_window() {
        // The same exchange with a realistic emulated timestamp, which is
        // what the hardware actually supplies.
        let (mut host, mut client) = connected_pair();
        host.send_cmd(0, &[0], 100_000);
        let mut buf = [0u8; 64];
        client.recv_host_packet(1, &mut buf);
        client.send_reply(1, &[7], 100_000, 1);

        let mut replies = vec![0u8; 15 * REPLY_SLOT_SIZE];
        host.set_recv_timeout(Duration::from_millis(20));
        assert_eq!(host.recv_replies(0, &mut replies, 100_000, 1 << 1), 1 << 1);
    }

    #[test]
    fn host_gone_is_reported_after_the_host_leaves() {
        let (mut host, mut client) = connected_pair();
        host.send_cmd(0, &[0], 100);
        let mut buf = [0u8; 64];
        // Receiving the CMD frame is what teaches the client who the host is.
        assert!(matches!(client.recv_host_packet(1, &mut buf), MpRecvResult::Frame { .. }));

        host.end(0);
        client.set_recv_timeout(Duration::from_millis(5));
        assert_eq!(client.recv_host_packet(1, &mut buf), MpRecvResult::HostGone);
    }

    #[test]
    fn oversized_frames_are_rejected() {
        let (mut host, _client) = connected_pair();
        let too_big = vec![0u8; MAX_FRAME_SIZE + 1];
        assert_eq!(host.send_packet(0, &too_big, 0), 0);
    }

    #[test]
    fn short_receive_buffer_truncates_without_desync() {
        let (mut host, mut client) = connected_pair();
        host.send_packet(0, &[1, 2, 3, 4, 5, 6], 1);
        host.send_packet(0, &[7, 8], 2);

        let mut small = [0u8; 2];
        assert!(matches!(
            client.recv_packet(1, &mut small),
            MpRecvResult::Frame { len: 2, timestamp: 1, .. }
        ));
        // The second frame must still parse: the first was fully consumed
        // from the FIFO even though only 2 bytes were handed back.
        let mut buf = [0u8; 16];
        assert!(matches!(
            client.recv_packet(1, &mut buf),
            MpRecvResult::Frame { len: 2, timestamp: 2, .. }
        ));
        assert_eq!(&buf[..2], &[7, 8]);
    }

    #[test]
    fn fifo_wraps_around_the_end_of_the_ring() {
        let (mut host, mut client) = connected_pair();
        // Each frame costs 24 header bytes + body; push well past 64 KiB
        // so the write cursor wraps, reading as we go so the reader wraps
        // too.
        let body = [0xA5u8; 512];
        let mut buf = [0u8; 512];
        for i in 0..200u64 {
            assert_eq!(host.send_packet(0, &body, i), body.len());
            let recv = client.recv_packet(1, &mut buf);
            assert_eq!(
                recv,
                MpRecvResult::Frame {
                    len: body.len(),
                    frame_type: MpFrameType::REGULAR,
                    timestamp: i,
                }
            );
            assert_eq!(buf, body);
        }
        let status = host.hub().status();
        assert!(status.packet_write_offset < PACKET_QUEUE_SIZE as u32);
    }

    #[test]
    fn begin_skips_frames_sent_before_joining() {
        let hub = Arc::new(LocalMpHub::new());
        let mut host = LocalMp::from_hub(Arc::clone(&hub));
        host.begin(0);
        host.send_packet(0, &[1, 2, 3], 1);

        let mut late = LocalMp::from_hub(Arc::clone(&hub));
        late.begin(1);
        let mut buf = [0u8; 16];
        assert_eq!(late.recv_packet(1, &mut buf), MpRecvResult::None);
    }

    #[test]
    fn reply_with_invalid_aid_is_dropped() {
        let (mut host, mut client) = connected_pair();
        host.send_cmd(0, &[0], 1_000);
        let mut buf = [0u8; 64];
        client.recv_host_packet(1, &mut buf);
        // aid 0 would underflow melonDS's `(aid - 1) * 1024` index.
        client.send_reply(1, &[1], 1_000, 0);

        let mut replies = vec![0u8; 15 * REPLY_SLOT_SIZE];
        host.set_recv_timeout(Duration::from_millis(5));
        assert_eq!(host.recv_replies(0, &mut replies, 1_000, 1 << 1), 0);
        assert!(replies.iter().all(|&b| b == 0));
    }
}
