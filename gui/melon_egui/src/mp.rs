//! In-process wireless: the airwaves two consoles in this window share.
//!
//! This is a Rust port of melonDS's `net/LocalMP.cpp`, which is what its own
//! "Launch new instance" uses. The semantics are the ones `shim.h` promises:
//! timestamps are the sender's emulated wifi microsecond clock, a receive
//! returns the packet length (0 = nothing available, -1 = not connected), and
//! `recv_replies` returns the bitmask of AIDs whose replies it wrote.
//!
//! # The shape of a DS wireless round
//!
//! Local play is not "everyone talks when they like". The host sends a **CMD**
//! frame naming the clients it wants to hear from; each client's hardware
//! answers with a **reply** the moment it receives that frame; the host then
//! sends an **ACK**. One CMD/reply/ACK round happens per game frame, and the
//! game's data rides on it. Beacons and the association handshake travel as
//! ordinary packets before any of that starts.
//!
//! Replies therefore live in their own queue, separate from ordinary packets:
//! the host drains them all at once, keyed by AID, in [`Airwaves::recv_replies`].
//!
//! # Differences from melonDS's LocalMP, and why
//!
//! melonDS shares its queues between *processes* (shared memory plus named
//! semaphores), because its instances are separate program launches. Here both
//! consoles live in one process, so a `Mutex` around plain `VecDeque`s does the
//! same job. The blocking receives melonDS implements with a semaphore timeout
//! are non-blocking here — see [`Airwaves::recv_host_packet`].

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// How many consoles can share these airwaves. Two is what "Launch new
/// instance" opens; the AID bookkeeping below allows more.
pub const MAX_INSTANCES: usize = 16;

/// A reply slot is 1024 bytes, and `recv_replies` is handed one buffer holding
/// all of them (melonDS `kMaxFrameSize` reasoning; the wrapper sizes its buffer
/// at 16 KiB for exactly this).
const REPLY_SLOT: usize = 1024;

/// How far behind the host's clock a reply may be and still count. melonDS uses
/// the same 32 microseconds in `RecvReplies`.
const STALE_MICROSECONDS: u64 = 32;

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
struct Packet {
    sender: usize,
    kind: Kind,
    timestamp: u64,
    data: Vec<u8>,
}

/// A running count of what each console has sent and received, which is what
/// the diagnostics window reports.
#[derive(Clone, Copy, Default)]
pub struct Counters {
    pub sent_generic: u64,
    pub sent_cmd: u64,
    pub sent_reply: u64,
    pub sent_ack: u64,
    pub recv_generic: u64,
    pub recv_cmd: u64,
    pub recv_reply: u64,
    /// Replies dropped for being older than the host's round.
    pub stale_replies: u64,
    /// The newest wifi clock this console reported.
    pub clock: u64,
    /// The last AID mask `recv_replies` returned, so a host that is asking but
    /// hearing nothing is visible.
    pub last_reply_mask: u16,
}

/// One console's queues.
#[derive(Default)]
struct Mailbox {
    packets: VecDeque<Packet>,
    replies: VecDeque<Packet>,
    counters: Counters,
    connected: bool,
}

#[derive(Default)]
struct Shared {
    boxes: Vec<Mailbox>,
    /// A short rolling history for the diagnostics window, newest last.
    log: VecDeque<Event>,
    /// Which console last sent a CMD, so a client can tell its host has gone.
    last_host: Option<usize>,
}

/// A line in the traffic log.
#[derive(Clone)]
pub struct Event {
    pub sender: usize,
    pub kind: Kind,
    pub timestamp: u64,
    pub len: usize,
}

/// How many log lines are kept.
const LOG_LIMIT: usize = 400;

/// The shared medium. Cheap to clone; every console holds one.
#[derive(Clone)]
pub struct Airwaves(Arc<Mutex<Shared>>);

impl Default for Airwaves {
    fn default() -> Self {
        Self::new()
    }
}

impl Airwaves {
    pub fn new() -> Self {
        let mut shared = Shared::default();
        shared.boxes.resize_with(MAX_INSTANCES, Mailbox::default);
        Self(Arc::new(Mutex::new(shared)))
    }

    /// A handle for console `instance`, to hand to [`crate::emu::Emu`].
    pub fn client(&self, instance: usize) -> Client {
        Client { airwaves: self.clone(), instance }
    }

    /// Per-console counters, for the diagnostics window.
    pub fn counters(&self) -> Vec<Counters> {
        let shared = self.0.lock().unwrap();
        shared.boxes.iter().map(|b| b.counters).collect()
    }

    /// Which consoles have called `mp_begin` and not `mp_end`.
    pub fn connected(&self) -> Vec<bool> {
        let shared = self.0.lock().unwrap();
        shared.boxes.iter().map(|b| b.connected).collect()
    }

    /// The rolling traffic log, oldest first.
    pub fn log(&self) -> Vec<Event> {
        let shared = self.0.lock().unwrap();
        shared.log.iter().cloned().collect()
    }

    pub fn clear_log(&self) {
        self.0.lock().unwrap().log.clear();
    }

    /// The bitmask of connected consoles, as melonDS's `ConnectedBitmask`.
    fn connected_mask(shared: &Shared) -> u16 {
        shared.boxes.iter().enumerate().fold(
            0u16,
            |mask, (i, b)| {
                if b.connected { mask | (1 << i) } else { mask }
            },
        )
    }
}

/// One console's view of the airwaves.
#[derive(Clone)]
pub struct Client {
    airwaves: Airwaves,
    instance: usize,
}

impl Client {
    fn begin(&self) {
        let mut shared = self.airwaves.0.lock().unwrap();
        shared.boxes[self.instance].connected = true;
    }

    fn end(&self) {
        let mut shared = self.airwaves.0.lock().unwrap();
        let mailbox = &mut shared.boxes[self.instance];
        mailbox.connected = false;
        // Anything still queued for a console that has left is not going to be
        // read, and would otherwise be delivered stale if it rejoins.
        mailbox.packets.clear();
        mailbox.replies.clear();
    }

    /// Broadcast to every *other* console, which is what a radio does. melonDS
    /// writes into a shared FIFO and filters the sender out on read; the effect
    /// is the same and this way a console never sees its own frame.
    fn send(&self, kind: Kind, data: &[u8], timestamp: u64) -> i32 {
        let mut shared = self.airwaves.0.lock().unwrap();

        let packet = Packet { sender: self.instance, kind, timestamp, data: data.to_vec() };
        if kind == Kind::Cmd {
            shared.last_host = Some(self.instance);
        }

        for other in 0..MAX_INSTANCES {
            if other == self.instance || !shared.boxes[other].connected {
                continue;
            }
            match kind {
                Kind::Reply(_) => shared.boxes[other].replies.push_back(packet.clone()),
                _ => shared.boxes[other].packets.push_back(packet.clone()),
            }
        }

        let counters = &mut shared.boxes[self.instance].counters;
        match kind {
            Kind::Generic => counters.sent_generic += 1,
            Kind::Cmd => counters.sent_cmd += 1,
            Kind::Reply(_) => counters.sent_reply += 1,
            Kind::Ack => counters.sent_ack += 1,
        }

        let event = Event { sender: self.instance, kind, timestamp, len: data.len() };
        shared.log.push_back(event);
        if shared.log.len() > LOG_LIMIT {
            shared.log.pop_front();
        }

        data.len() as i32
    }

    /// Take the next ordinary packet, if one is waiting.
    fn recv(&self, out: &mut [u8], timestamp: &mut u64) -> i32 {
        let mut shared = self.airwaves.0.lock().unwrap();
        let Some(packet) = shared.boxes[self.instance].packets.pop_front() else {
            return 0;
        };
        let len = packet.data.len().min(out.len());
        out[..len].copy_from_slice(&packet.data[..len]);
        *timestamp = packet.timestamp;

        let counters = &mut shared.boxes[self.instance].counters;
        match packet.kind {
            Kind::Cmd => counters.recv_cmd += 1,
            _ => counters.recv_generic += 1,
        }
        len as i32
    }
}

impl melonds::Host for Client {
    fn mp_begin(&self) {
        self.begin();
    }

    fn mp_end(&self) {
        self.end();
    }

    fn mp_send_packet(&self, data: &[u8], timestamp: u64) -> i32 {
        self.send(Kind::Generic, data, timestamp)
    }

    fn mp_send_cmd(&self, data: &[u8], timestamp: u64) -> i32 {
        self.send(Kind::Cmd, data, timestamp)
    }

    fn mp_send_reply(&self, data: &[u8], timestamp: u64, aid: u16) -> i32 {
        self.send(Kind::Reply(aid), data, timestamp)
    }

    fn mp_send_ack(&self, data: &[u8], timestamp: u64) -> i32 {
        self.send(Kind::Ack, data, timestamp)
    }

    fn mp_recv_packet(&self, data: &mut [u8], _now: u64, timestamp: &mut u64) -> Option<i32> {
        Some(self.recv(data, timestamp))
    }

    /// As [`Self::mp_recv_packet`], but reports `-1` once the console whose CMDs
    /// this one has been following is no longer connected — which is how a
    /// client learns its host has gone rather than waiting forever.
    fn mp_recv_host_packet(&self, data: &mut [u8], _now: u64, timestamp: &mut u64) -> Option<i32> {
        {
            let shared = self.airwaves.0.lock().unwrap();
            if let Some(host) = shared.last_host
                && host != self.instance
                && !shared.boxes[host].connected
            {
                return Some(-1);
            }
        }
        Some(self.recv(data, timestamp))
    }

    /// Drain the reply queue into per-AID slots, returning the mask of AIDs that
    /// answered.
    ///
    /// Ported from melonDS's `RecvReplies`: replies from this console itself and
    /// replies older than the round are skipped, each reply is written at
    /// `(aid - 1) * 1024`, and the drain stops early once every connected
    /// console — or everyone the caller asked for — has been heard from.
    fn mp_recv_replies(&self, data: &mut [u8], _now: u64, timestamp: u64, aidmask: u16) -> u16 {
        let mut shared = self.airwaves.0.lock().unwrap();
        let connected = Airwaves::connected_mask(&shared);
        let mut seen = 1u16 << self.instance;
        // Nobody else is on the air, so there is nothing to wait for.
        if seen & connected == connected {
            return 0;
        }

        let mut mask = 0u16;
        while let Some(packet) = shared.boxes[self.instance].replies.pop_front() {
            let Kind::Reply(aid) = packet.kind else {
                continue;
            };
            // `timestamp` is the round's; anything older belongs to a round that
            // has already been answered.
            if packet.sender == self.instance || packet.timestamp + STALE_MICROSECONDS < timestamp {
                shared.boxes[self.instance].counters.stale_replies += 1;
                continue;
            }

            if !packet.data.is_empty() && aid >= 1 {
                let start = (aid as usize - 1) * REPLY_SLOT;
                if let Some(slot) = data.get_mut(start..start + packet.data.len()) {
                    slot.copy_from_slice(&packet.data);
                    mask |= 1 << aid;
                }
            }
            shared.boxes[self.instance].counters.recv_reply += 1;

            seen |= 1 << packet.sender;
            if seen & connected == connected || (mask & aidmask) == aidmask {
                break;
            }
        }

        shared.boxes[self.instance].counters.last_reply_mask = mask;
        mask
    }

    fn mp_clock(&self, now: u64) {
        let mut shared = self.airwaves.0.lock().unwrap();
        shared.boxes[self.instance].counters.clock = now;
    }
}

#[cfg(test)]
mod tests {
    use melonds::Host;

    use super::{Airwaves, Kind};

    /// Two consoles, both on the air.
    fn pair() -> (Airwaves, super::Client, super::Client) {
        let air = Airwaves::new();
        let (a, b) = (air.client(0), air.client(1));
        a.mp_begin();
        b.mp_begin();
        (air, a, b)
    }

    #[test]
    fn a_packet_reaches_the_other_console_and_not_the_sender() {
        let (_air, a, b) = pair();
        a.mp_send_packet(b"beacon", 1000);

        let mut buf = [0u8; 64];
        let mut ts = 0;
        // The sender must not hear itself.
        assert_eq!(a.mp_recv_packet(&mut buf, 0, &mut ts), Some(0));

        let len = b.mp_recv_packet(&mut buf, 0, &mut ts).unwrap();
        assert_eq!(len, 6);
        assert_eq!(&buf[..6], b"beacon");
        assert_eq!(ts, 1000, "the sender's wifi clock rides with the frame");
    }

    #[test]
    fn nothing_is_delivered_to_a_console_that_has_not_joined() {
        let air = Airwaves::new();
        let (a, b) = (air.client(0), air.client(1));
        a.mp_begin(); // b never joins
        a.mp_send_packet(b"beacon", 1000);

        let mut buf = [0u8; 64];
        let mut ts = 0;
        assert_eq!(b.mp_recv_packet(&mut buf, 0, &mut ts), Some(0));
    }

    #[test]
    fn a_reply_lands_in_the_slot_for_its_aid() {
        let (_air, host, client) = pair();
        host.mp_send_cmd(b"cmd", 5000);
        client.mp_send_reply(b"hello", 5010, 1);

        let mut buf = vec![0u8; 16 * 1024];
        let mask = host.mp_recv_replies(&mut buf, 0, 5000, 0b10);
        assert_eq!(mask, 0b10, "AID 1 answered");
        // AID 1 writes at (1 - 1) * 1024.
        assert_eq!(&buf[..5], b"hello");
    }

    #[test]
    fn a_reply_from_an_earlier_round_is_dropped() {
        let (air, host, client) = pair();
        // The reply is timestamped well before the round the host is asking
        // about, so it belongs to a round already finished.
        client.mp_send_reply(b"late", 1000, 1);

        let mut buf = vec![0u8; 16 * 1024];
        let mask = host.mp_recv_replies(&mut buf, 0, 9000, 0b10);
        assert_eq!(mask, 0, "a stale reply must not be counted");
        assert_eq!(air.counters()[0].stale_replies, 1);
    }

    #[test]
    fn a_reply_just_inside_the_tolerance_still_counts() {
        let (_air, host, client) = pair();
        // melonDS allows 32 microseconds of slack either way.
        client.mp_send_reply(b"ok", 5000 - 32, 1);
        let mut buf = vec![0u8; 16 * 1024];
        assert_eq!(host.mp_recv_replies(&mut buf, 0, 5000, 0b10), 0b10);
    }

    #[test]
    fn replies_do_not_come_back_as_ordinary_packets() {
        let (_air, host, client) = pair();
        client.mp_send_reply(b"hello", 5000, 1);

        let mut buf = [0u8; 64];
        let mut ts = 0;
        assert_eq!(
            host.mp_recv_packet(&mut buf, 0, &mut ts),
            Some(0),
            "a reply belongs to the reply queue only",
        );
    }

    #[test]
    fn a_client_learns_its_host_has_gone() {
        let (_air, host, client) = pair();
        host.mp_send_cmd(b"cmd", 5000);

        let mut buf = [0u8; 64];
        let mut ts = 0;
        // The CMD arrives normally while the host is up.
        assert_eq!(client.mp_recv_host_packet(&mut buf, 0, &mut ts), Some(3));

        host.mp_end();
        assert_eq!(
            client.mp_recv_host_packet(&mut buf, 0, &mut ts),
            Some(-1),
            "-1 is how the core is told the host left",
        );
    }

    #[test]
    fn a_lone_console_is_told_there_are_no_replies_coming() {
        let air = Airwaves::new();
        let host = air.client(0);
        host.mp_begin();
        let mut buf = vec![0u8; 16 * 1024];
        assert_eq!(host.mp_recv_replies(&mut buf, 0, 1000, 0b10), 0);
    }

    #[test]
    fn traffic_is_counted_and_logged_by_kind() {
        let (air, host, client) = pair();
        host.mp_send_packet(b"beacon", 100);
        host.mp_send_cmd(b"cmd", 200);
        client.mp_send_reply(b"r", 210, 1);
        host.mp_send_ack(b"ack", 220);

        let counters = air.counters();
        assert_eq!(counters[0].sent_generic, 1);
        assert_eq!(counters[0].sent_cmd, 1);
        assert_eq!(counters[0].sent_ack, 1);
        assert_eq!(counters[1].sent_reply, 1);

        let log = air.log();
        assert_eq!(log.len(), 4);
        assert_eq!(log[1].kind, Kind::Cmd);
        assert_eq!(log[2].kind, Kind::Reply(1));
        assert_eq!(log[3].timestamp, 220);
    }

    #[test]
    fn leaving_the_air_drops_what_was_queued_for_that_console() {
        let (_air, a, b) = pair();
        a.mp_send_packet(b"beacon", 100);
        b.mp_end();
        b.mp_begin();

        let mut buf = [0u8; 64];
        let mut ts = 0;
        assert_eq!(
            b.mp_recv_packet(&mut buf, 0, &mut ts),
            Some(0),
            "a rejoining console must not receive frames from before it left",
        );
    }

    #[test]
    fn the_clock_hook_records_each_console_s_wifi_time() {
        let (air, a, b) = pair();
        a.mp_clock(12_345);
        b.mp_clock(12_400);
        let counters = air.counters();
        assert_eq!(counters[0].clock, 12_345);
        assert_eq!(counters[1].clock, 12_400);
    }
}
