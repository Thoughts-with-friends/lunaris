//! Fan-out of Ethernet frames to per-instance receive queues.
//!
//! Port of melonDS's `src/net/PacketDispatcher.{h,cpp}`, including its
//! backing `RingBuffer<0x8000>` from `FIFO.h`. Each registered instance
//! owns a 32 KiB ring; a send writes one copy of the frame into every
//! queue named by the receive mask, and the oldest frames are discarded
//! when a queue is too full to fit the new one.
//!
//! Unlike melonDS, all state sits behind one mutex owned by the
//! dispatcher, so [`PacketDispatcher`] takes `&self` throughout and can be
//! shared through an [`std::sync::Arc`] with a [`super::NetDriver`]'s
//! receive callback.

use std::sync::{Mutex, MutexGuard};

use crate::hw::net::mp_interface::MAX_INSTANCES;

/// Capacity of one instance's receive ring, in bytes.
pub const PACKET_QUEUE_SIZE: usize = 0x8000;

/// `'MPLK'` — tags each queued packet so a desynchronised reader bails out
/// rather than returning garbage.
const PACKET_MAGIC: u32 = 0x4B50_4C4D;

/// Pseudo-sender used for frames that arrived from outside the emulator
/// (see [`super::Net::rx_enqueue`]). It is deliberately outside the
/// `0..16` instance range so that the "don't echo to the sender" rule
/// never removes a real instance from the receive mask.
pub const EXTERNAL_SENDER: u8 = 16;

/// Per-packet header stored inline in the ring, ahead of the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PacketHeader {
    magic: u32,
    sender_id: u32,
    header_length: u32,
    data_length: u32,
}

impl PacketHeader {
    const ENCODED_LEN: usize = 16;

    fn to_bytes(self) -> [u8; Self::ENCODED_LEN] {
        let mut out = [0u8; Self::ENCODED_LEN];
        out[0..4].copy_from_slice(&self.magic.to_le_bytes());
        out[4..8].copy_from_slice(&self.sender_id.to_le_bytes());
        out[8..12].copy_from_slice(&self.header_length.to_le_bytes());
        out[12..16].copy_from_slice(&self.data_length.to_le_bytes());
        out
    }

    fn from_bytes(bytes: &[u8; Self::ENCODED_LEN]) -> Self {
        let word = |lo: usize| -> u32 {
            u32::from_le_bytes([bytes[lo], bytes[lo + 1], bytes[lo + 2], bytes[lo + 3]])
        };
        PacketHeader {
            magic: word(0),
            sender_id: word(4),
            header_length: word(8),
            data_length: word(12),
        }
    }
}

/// Byte-oriented ring buffer, the equivalent of melonDS's
/// `RingBuffer<0x8000>`.
///
/// Reads fail (returning `false`) rather than under-running when fewer
/// bytes are stored than requested.
struct PacketQueue {
    data: Box<[u8]>,
    read_offset: usize,
    write_offset: usize,
    filled: usize,
}

impl PacketQueue {
    fn new() -> Self {
        PacketQueue {
            data: vec![0u8; PACKET_QUEUE_SIZE].into_boxed_slice(),
            read_offset: 0,
            write_offset: 0,
            filled: 0,
        }
    }

    const fn clear(&mut self) {
        self.read_offset = 0;
        self.write_offset = 0;
        self.filled = 0;
    }

    /// `true` if `len` more bytes fit without evicting anything.
    const fn can_fit(&self, len: usize) -> bool {
        self.filled + len <= self.data.len()
    }

    fn write(&mut self, buf: &[u8]) {
        debug_assert!(self.can_fit(buf.len()), "packet queue overrun");
        let capacity = self.data.len();
        for &byte in buf {
            self.data[self.write_offset] = byte;
            self.write_offset = (self.write_offset + 1) % capacity;
        }
        self.filled += buf.len();
    }

    fn read(&mut self, buf: &mut [u8]) -> bool {
        if self.filled < buf.len() {
            return false;
        }
        let capacity = self.data.len();
        for slot in buf.iter_mut() {
            *slot = self.data[self.read_offset];
            self.read_offset = (self.read_offset + 1) % capacity;
        }
        self.filled -= buf.len();
        true
    }

    fn skip(&mut self, len: usize) -> bool {
        if self.filled < len {
            return false;
        }
        self.read_offset = (self.read_offset + len) % self.data.len();
        self.filled -= len;
        true
    }
}

/// Sizes of the two parts of a packet returned by
/// [`PacketDispatcher::recv_packet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchedPacket {
    /// Bytes the sender supplied as the header part.
    pub header_len: usize,
    /// Bytes the sender supplied as the data part.
    pub data_len: usize,
}

/// Mutex-guarded dispatcher state.
struct Queues {
    instance_mask: u16,
    queues: [Option<PacketQueue>; MAX_INSTANCES],
}

/// Routes packets from any sender to the registered instances named by a
/// receive mask.
pub struct PacketDispatcher {
    inner: Mutex<Queues>,
}

impl Default for PacketDispatcher {
    fn default() -> Self {
        PacketDispatcher::new()
    }
}

impl PacketDispatcher {
    /// Creates a dispatcher with no instance registered.
    #[must_use]
    pub const fn new() -> Self {
        PacketDispatcher {
            inner: Mutex::new(Queues { instance_mask: 0, queues: [const { None }; MAX_INSTANCES] }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Queues> {
        // Recovering from a poisoned lock is safe: a torn write leaves a
        // queue whose next header read fails the magic check, which
        // `recv_packet` already handles by returning `None`.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Gives instance `inst` a receive queue. Port of
    /// `PacketDispatcher::registerInstance`.
    pub fn register_instance(&self, inst: u8) {
        let Some(index) = instance_index(inst) else { return };
        let mut inner = self.lock();
        inner.instance_mask |= 1 << index;
        inner.queues[index] = Some(PacketQueue::new());
    }

    /// Drops instance `inst`'s receive queue. Port of
    /// `PacketDispatcher::unregisterInstance`.
    pub fn unregister_instance(&self, inst: u8) {
        let Some(index) = instance_index(inst) else { return };
        let mut inner = self.lock();
        inner.instance_mask &= !(1 << index);
        inner.queues[index] = None;
    }

    /// Empties every registered queue. Port of `PacketDispatcher::clear`.
    pub fn clear(&self) {
        let mut inner = self.lock();
        for queue in inner.queues.iter_mut().flatten() {
            queue.clear();
        }
    }

    /// Copies a packet into the queue of every registered instance in
    /// `recv_mask`.
    ///
    /// Port of `PacketDispatcher::sendPacket`. `sender` may be
    /// [`EXTERNAL_SENDER`] for frames from outside the emulator; a sender
    /// in `0..16` is removed from the receive mask so an instance never
    /// receives its own transmission.
    pub fn send_packet(
        &self,
        header: Option<&[u8]>,
        data: Option<&[u8]>,
        sender: u8,
        recv_mask: u16,
    ) {
        let header = header.unwrap_or(&[]);
        let data = data.unwrap_or(&[]);
        if header.is_empty() && data.is_empty() {
            return;
        }
        let total_len = PacketHeader::ENCODED_LEN + header.len() + data.len();
        if total_len >= PACKET_QUEUE_SIZE {
            return;
        }
        if sender as usize > MAX_INSTANCES {
            return;
        }

        let mut inner = self.lock();
        let mut recv_mask = recv_mask & inner.instance_mask;
        if let Some(index) = instance_index(sender) {
            recv_mask &= !(1 << index);
        }
        if recv_mask == 0 {
            return;
        }

        let packet_header = PacketHeader {
            magic: PACKET_MAGIC,
            sender_id: u32::from(sender),
            header_length: header.len() as u32,
            data_length: data.len() as u32,
        };
        let header_bytes = packet_header.to_bytes();

        for (i, queue) in inner.queues.iter_mut().enumerate() {
            if recv_mask & (1 << i) == 0 {
                continue;
            }
            let Some(queue) = queue.as_mut() else { continue };

            // Out of room: drop whole packets from the front until the new
            // one fits.
            while !queue.can_fit(total_len) {
                let mut stale = [0u8; PacketHeader::ENCODED_LEN];
                if !queue.read(&mut stale) {
                    // Nothing left to evict; the queue is now empty and
                    // `total_len` is known to fit an empty queue.
                    queue.clear();
                    break;
                }
                let stale = PacketHeader::from_bytes(&stale);
                if !queue.skip(stale.header_length as usize + stale.data_length as usize) {
                    queue.clear();
                    break;
                }
            }

            queue.write(&header_bytes);
            if !header.is_empty() {
                queue.write(header);
            }
            if !data.is_empty() {
                queue.write(data);
            }
        }
    }

    /// Pops the oldest packet queued for `receiver`, copying its header and
    /// data parts into the supplied buffers.
    ///
    /// Port of `PacketDispatcher::recvPacket`. Passing `None` for either
    /// part skips it in the queue rather than copying it, and a buffer
    /// shorter than the stored part is filled as far as it goes while the
    /// rest is still consumed.
    ///
    /// Returns `None` when nothing is queued, `receiver` is not registered,
    /// or the queue lost synchronisation.
    pub fn recv_packet(
        &self,
        header: Option<&mut [u8]>,
        data: Option<&mut [u8]>,
        receiver: u8,
    ) -> Option<DispatchedPacket> {
        if header.is_none() && data.is_none() {
            return None;
        }
        let index = instance_index(receiver)?;

        let mut inner = self.lock();
        let queue = inner.queues[index].as_mut()?;

        let mut header_bytes = [0u8; PacketHeader::ENCODED_LEN];
        if !queue.read(&mut header_bytes) {
            return None;
        }
        let packet_header = PacketHeader::from_bytes(&header_bytes);
        if packet_header.magic != PACKET_MAGIC {
            return None;
        }

        let header_len = packet_header.header_length as usize;
        let data_len = packet_header.data_length as usize;
        if header_len != 0 {
            read_part(queue, header, header_len);
        }
        if data_len != 0 {
            read_part(queue, data, data_len);
        }

        Some(DispatchedPacket { header_len, data_len })
    }
}

/// Copies `len` bytes out of the queue into `buf` (as much as fits) and
/// discards the remainder, so the queue stays aligned to packet boundaries
/// whatever the caller's buffer size.
fn read_part(queue: &mut PacketQueue, buf: Option<&mut [u8]>, len: usize) {
    let copied = match buf {
        Some(buf) => {
            let copied = len.min(buf.len());
            queue.read(&mut buf[..copied]);
            copied
        }
        None => 0,
    };
    if copied < len {
        queue.skip(len - copied);
    }
}

/// Validates an instance index against the 16-entry instance mask.
const fn instance_index(inst: u8) -> Option<usize> {
    if (inst as usize) < MAX_INSTANCES { Some(inst as usize) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_reaches_every_registered_receiver_but_not_the_sender() {
        let dispatcher = PacketDispatcher::new();
        dispatcher.register_instance(0);
        dispatcher.register_instance(1);
        dispatcher.send_packet(None, Some(&[1, 2, 3]), 0, 0xFFFF);

        let mut buf = [0u8; 16];
        let recv = dispatcher.recv_packet(None, Some(&mut buf), 1);
        assert_eq!(recv, Some(DispatchedPacket { header_len: 0, data_len: 3 }));
        assert_eq!(&buf[..3], &[1, 2, 3]);
        assert_eq!(dispatcher.recv_packet(None, Some(&mut buf), 0), None);
    }

    #[test]
    fn external_sender_is_not_masked_out() {
        let dispatcher = PacketDispatcher::new();
        dispatcher.register_instance(0);
        // Sender 16 is outside the instance range, so instance 0 must
        // still receive the frame.
        dispatcher.send_packet(None, Some(&[7]), EXTERNAL_SENDER, 0xFFFF);

        let mut buf = [0u8; 4];
        assert_eq!(
            dispatcher.recv_packet(None, Some(&mut buf), 0),
            Some(DispatchedPacket { header_len: 0, data_len: 1 })
        );
        assert_eq!(buf[0], 7);
    }

    #[test]
    fn unregistered_receiver_yields_nothing() {
        let dispatcher = PacketDispatcher::new();
        let mut buf = [0u8; 4];
        assert_eq!(dispatcher.recv_packet(None, Some(&mut buf), 3), None);
    }

    #[test]
    fn header_and_data_parts_round_trip_separately() {
        let dispatcher = PacketDispatcher::new();
        dispatcher.register_instance(1);
        dispatcher.send_packet(Some(&[0xAA, 0xBB]), Some(&[1, 2, 3, 4]), 0, 0xFFFF);

        let mut header = [0u8; 2];
        let mut data = [0u8; 4];
        let recv = dispatcher.recv_packet(Some(&mut header), Some(&mut data), 1);
        assert_eq!(recv, Some(DispatchedPacket { header_len: 2, data_len: 4 }));
        assert_eq!(header, [0xAA, 0xBB]);
        assert_eq!(data, [1, 2, 3, 4]);
    }

    #[test]
    fn oldest_packets_are_evicted_when_the_queue_fills() {
        let dispatcher = PacketDispatcher::new();
        dispatcher.register_instance(1);

        // Each packet costs 16 header bytes + 1024 data bytes, so ~31 fit
        // in a 32 KiB ring; 64 of them force eviction.
        let body = [0x5Au8; 1024];
        for _ in 0..64 {
            dispatcher.send_packet(None, Some(&body), 0, 0xFFFF);
        }

        // Whatever survived must still parse cleanly, packet by packet.
        let mut buf = [0u8; 1024];
        let mut received = 0;
        while let Some(packet) = dispatcher.recv_packet(None, Some(&mut buf), 1) {
            assert_eq!(packet.data_len, 1024);
            assert_eq!(buf, body);
            received += 1;
        }
        assert!(received > 0, "eviction must not empty the queue entirely");
    }

    #[test]
    fn oversized_packets_are_dropped() {
        let dispatcher = PacketDispatcher::new();
        dispatcher.register_instance(1);
        let huge = vec![0u8; PACKET_QUEUE_SIZE];
        dispatcher.send_packet(None, Some(&huge), 0, 0xFFFF);

        let mut buf = [0u8; 16];
        assert_eq!(dispatcher.recv_packet(None, Some(&mut buf), 1), None);
    }

    #[test]
    fn clear_empties_every_queue() {
        let dispatcher = PacketDispatcher::new();
        dispatcher.register_instance(1);
        dispatcher.send_packet(None, Some(&[1]), 0, 0xFFFF);
        dispatcher.clear();

        let mut buf = [0u8; 4];
        assert_eq!(dispatcher.recv_packet(None, Some(&mut buf), 1), None);
    }
}
