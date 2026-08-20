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
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
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

/// How long a blocking receive waits, in *wall* time. melonDS's
/// `MPInterface::RecvTimeout` is 25 ms and its receives wait on a semaphore for
/// that long; this waits on a condvar instead, for the same reason and the same
/// duration — the peer is another thread now, and the answer is expected while
/// we wait.
const RECV_TIMEOUT: Duration = Duration::from_millis(25);

/// How stale a peer's last activity may be before a blocking receive stops
/// waiting for it.
///
/// A console that is paused, stopped, or still booting cannot answer, and
/// waiting out the full timeout on every round for one that never will is what
/// turns "the second window is paused" into a front end running at two frames a
/// second. Generous enough that a console merely running slowly still counts.
const PEER_TIMEOUT: Duration = Duration::from_millis(250);

/// The largest frame the wifi hardware moves, melonDS's `kMaxFrameSize`.
/// `SendPacketGeneric` refuses anything bigger and warns rather than truncating
/// it into the queue, and so does this.
const MAX_FRAME_SIZE: usize = 0x948;

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
    /// The console whose CMD frame this one last *received*, which is what
    /// melonDS's `LastHostID` is: it is set on receive, per instance, so that
    /// "my host has gone" is answered from what this console has actually
    /// heard rather than from who spoke last on the medium.
    last_host: Option<usize>,
    /// When this console last did anything on the air. `None` until it does.
    /// See [`PEER_TIMEOUT`].
    active: Option<Instant>,
}

#[derive(Default)]
struct Shared {
    boxes: Vec<Mailbox>,
    /// A short rolling history for the diagnostics window, newest last.
    log: VecDeque<Event>,
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
///
/// The condvar is what melonDS's per-instance semaphores are: a sender wakes
/// whoever is waiting for something to arrive. One for the medium rather than
/// one per console — with two consoles the difference is a spurious wake-up
/// nobody notices, and the waiters re-check what they are waiting for anyway.
#[derive(Clone)]
pub struct Airwaves(Arc<(Mutex<Shared>, Condvar)>);

impl Default for Airwaves {
    fn default() -> Self {
        Self::new()
    }
}

impl Airwaves {
    pub fn new() -> Self {
        let mut shared = Shared::default();
        shared.boxes.resize_with(MAX_INSTANCES, Mailbox::default);
        Self(Arc::new((Mutex::new(shared), Condvar::new())))
    }

    /// A handle for console `instance`, to hand to [`crate::emu::Emu`].
    pub fn client(&self, instance: usize) -> Client {
        Client { airwaves: self.clone(), instance }
    }

    /// Per-console counters, for the diagnostics window.
    pub fn counters(&self) -> Vec<Counters> {
        let shared = self.0.0.lock().unwrap();
        shared.boxes.iter().map(|b| b.counters).collect()
    }

    /// Which consoles have called `mp_begin` and not `mp_end`.
    pub fn connected(&self) -> Vec<bool> {
        let shared = self.0.0.lock().unwrap();
        shared.boxes.iter().map(|b| b.connected).collect()
    }

    /// The rolling traffic log, oldest first.
    pub fn log(&self) -> Vec<Event> {
        let shared = self.0.0.lock().unwrap();
        shared.log.iter().cloned().collect()
    }

    pub fn clear_log(&self) {
        self.0.0.lock().unwrap().log.clear();
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
        let mut shared = self.airwaves.0.0.lock().unwrap();
        shared.boxes[self.instance].connected = true;
    }

    fn end(&self) {
        let mut shared = self.airwaves.0.0.lock().unwrap();
        let mailbox = &mut shared.boxes[self.instance];
        mailbox.connected = false;
        // Anything still queued for a console that has left is not going to be
        // read, and would otherwise be delivered stale if it rejoins.
        mailbox.packets.clear();
        mailbox.replies.clear();
        mailbox.last_host = None;
    }

    /// Broadcast to every *other* console, which is what a radio does. melonDS
    /// writes into a shared FIFO and filters the sender out on read; the effect
    /// is the same and this way a console never sees its own frame.
    fn send(&self, kind: Kind, data: &[u8], timestamp: u64) -> i32 {
        // melonDS's SendPacketGeneric refuses an oversized frame outright:
        // truncating one into the queue would be read back as a frame whose
        // header disagrees with its length, which is worse than losing it.
        if data.len() > MAX_FRAME_SIZE {
            log::warn!("mp: refusing a {}-byte frame (max {MAX_FRAME_SIZE})", data.len());
            return 0;
        }

        let mut shared = self.airwaves.0.0.lock().unwrap();

        let packet = Packet { sender: self.instance, kind, timestamp, data: data.to_vec() };
        if kind == Kind::Cmd {
            // A CMD opens a new round, and melonDS empties the host's reply
            // queue here (`ReplyReadOffset = ReplyWriteOffset`, then
            // `Semaphore_Reset`). Anything still in it answers a round that is
            // over, and would otherwise be read as an answer to this one.
            shared.boxes[self.instance].replies.clear();
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

        // `RUST_LOG=debug` turns this into a running account of the
        // handshake, which is the only way to see how far a pair got before
        // one of them gave up.
        log::debug!(
            "mp: {} sent {} len={} ts={timestamp}",
            self.instance,
            kind.label(),
            data.len()
        );

        let event = Event { sender: self.instance, kind, timestamp, len: data.len() };
        shared.log.push_back(event);
        if shared.log.len() > LOG_LIMIT {
            shared.log.pop_front();
        }

        shared.boxes[self.instance].active = Some(Instant::now());
        drop(shared);
        // Whoever is blocked waiting for this: melonDS posts the receiving
        // instances' semaphores here, and for the same reason.
        self.airwaves.0.1.notify_all();

        data.len() as i32
    }

    /// Whether any *other* console could still answer.
    ///
    /// Blocking only makes sense against a peer that is connected and actually
    /// executing; one that is paused, stopped or still booting would cost a
    /// full timeout per round and give nothing back.
    fn peer_is_live(shared: &Shared, me: usize) -> bool {
        shared.boxes.iter().enumerate().any(|(i, mailbox)| {
            i != me
                && mailbox.connected
                && mailbox.active.is_some_and(|at| at.elapsed() < PEER_TIMEOUT)
        })
    }

    /// Take the next ordinary packet, if one is waiting. `now` is the
    /// receiving console's own wifi clock, for the trace only.
    fn recv(&self, out: &mut [u8], now: u64, timestamp: &mut u64) -> i32 {
        let mut shared = self.airwaves.0.0.lock().unwrap();
        let Some(packet) = shared.boxes[self.instance].packets.pop_front() else {
            return 0;
        };
        let len = packet.data.len().min(out.len());
        out[..len].copy_from_slice(&packet.data[..len]);
        *timestamp = packet.timestamp;

        log::debug!(
            "mp: {} received {} len={len} ts={} (now={now})",
            self.instance,
            packet.kind.label(),
            packet.timestamp
        );

        // melonDS sets LastHostID when a CMD frame is *received*, which is
        // what makes "the host has gone" answerable per console.
        if packet.kind == Kind::Cmd {
            shared.boxes[self.instance].last_host = Some(packet.sender);
        }

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

    fn mp_recv_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        Some(self.recv(data, now, timestamp))
    }

    /// As [`Self::mp_recv_packet`], but reports `-1` once the console whose CMDs
    /// this one has been following is no longer connected — which is how a
    /// client learns its host has gone rather than waiting forever.
    fn mp_recv_host_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        {
            let shared = self.airwaves.0.0.lock().unwrap();
            if let Some(host) = shared.boxes[self.instance].last_host
                && host != self.instance
                && !shared.boxes[host].connected
            {
                return Some(-1);
            }
        }

        // This is the one receive melonDS blocks on (`RecvPacketGeneric` with
        // `block = true`): a client waiting on its host's frame expects it to
        // arrive while it waits, which it does now that the two consoles run
        // on threads of their own.
        let deadline = Instant::now() + RECV_TIMEOUT;
        loop {
            let len = self.recv(data, now, timestamp);
            if len != 0 {
                return Some(len);
            }
            if !self.wait_for_traffic(deadline) {
                return Some(0);
            }
        }
    }

    /// Drain the reply queue into per-AID slots, returning the mask of AIDs that
    /// answered.
    ///
    /// Ported from melonDS's `RecvReplies`: replies from this console itself and
    /// replies older than the round are skipped, each reply is written at
    /// `(aid - 1) * 1024`, and the drain stops early once every connected
    /// console — or everyone the caller asked for — has been heard from.
    fn mp_recv_replies(&self, data: &mut [u8], _now: u64, timestamp: u64, aidmask: u16) -> u16 {
        let deadline = Instant::now() + RECV_TIMEOUT;
        let mut mask = 0u16;
        let mut seen = 1u16 << self.instance;
        loop {
            let done = self.collect_replies(data, timestamp, aidmask, &mut mask, &mut seen);
            if done || !self.wait_for_traffic(deadline) {
                break;
            }
        }
        self.airwaves.0.0.lock().unwrap().boxes[self.instance].counters.last_reply_mask = mask;
        mask
    }

    fn mp_clock(&self, now: u64) {
        let mut shared = self.airwaves.0.0.lock().unwrap();
        shared.boxes[self.instance].counters.clock = now;
        // The clock advancing is this console saying it is still executing,
        // which is what a peer's blocking receive waits on.
        shared.boxes[self.instance].active = Some(Instant::now());
    }
}

impl Client {
    /// Wait for anything to arrive, up to `deadline`. Returns whether waiting
    /// is still worth it — false once the deadline has passed or no peer is in
    /// a position to answer.
    fn wait_for_traffic(&self, deadline: Instant) -> bool {
        let (lock, condvar) = &*self.airwaves.0;
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        let shared = lock.lock().unwrap();
        if !Self::peer_is_live(&shared, self.instance) {
            return false;
        }
        let (_guard, result) = condvar.wait_timeout(shared, remaining).unwrap();
        !result.timed_out()
    }

    /// One pass over whatever replies have arrived, returning whether the round
    /// is answered — every connected console has spoken, or every aid the
    /// caller asked for has.
    ///
    /// Ported from melonDS's `RecvReplies`: replies from this console itself
    /// and replies older than the round are skipped, and each reply is written
    /// at `(aid - 1) * 1024`.
    fn collect_replies(
        &self,
        data: &mut [u8],
        timestamp: u64,
        aidmask: u16,
        mask: &mut u16,
        seen: &mut u16,
    ) -> bool {
        let mut shared = self.airwaves.0.0.lock().unwrap();
        let connected = Airwaves::connected_mask(&shared);
        // Nobody else is on the air, so there is nothing to wait for.
        if *seen & connected == connected {
            return true;
        }

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
                    *mask |= 1 << aid;
                }
            }
            shared.boxes[self.instance].counters.recv_reply += 1;

            *seen |= 1 << packet.sender;
            if *seen & connected == connected || (*mask & aidmask) == aidmask {
                return true;
            }
        }
        false
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

    /// The whole point of the second console having a thread: a receive that
    /// blocks is answered by a peer that is running *now*.
    #[test]
    fn a_blocking_receive_is_answered_by_a_peer_that_is_still_running() {
        let (_air, a, b) = pair();
        // The peer has to look alive, or waiting for it is refused outright.
        a.mp_clock(1_000);

        let sender = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(5));
            a.mp_send_cmd(b"round", 2_000);
        });

        let started = std::time::Instant::now();
        let mut buf = [0u8; 64];
        let mut ts = 0;
        let len = b.mp_recv_host_packet(&mut buf, 0, &mut ts).unwrap();
        let waited = started.elapsed();
        sender.join().unwrap();

        assert_eq!(len, 5, "the CMD arrived while the receive was waiting");
        assert!(waited < super::RECV_TIMEOUT, "it returned on the packet, not on the timeout");
        assert!(waited >= std::time::Duration::from_millis(4), "it really did wait");
    }

    /// And the other half: waiting on a console that is not executing would
    /// cost a full timeout every round, so it is not done at all.
    #[test]
    fn nothing_waits_on_a_peer_that_is_not_running() {
        let (_air, _a, b) = pair();
        // `a` never says anything, so it has no activity to be within
        // PEER_TIMEOUT of.
        let started = std::time::Instant::now();
        let mut buf = [0u8; 64];
        let mut ts = 0;
        assert_eq!(b.mp_recv_host_packet(&mut buf, 0, &mut ts), Some(0));
        assert!(started.elapsed() < std::time::Duration::from_millis(5), "returned at once");
    }

    /// The host's reply collection waits the same way, which is what a
    /// wireless round needs: the answer is produced by the other console after
    /// the CMD goes out, and the host is still asking when it arrives.
    #[test]
    fn a_reply_that_arrives_late_is_still_collected() {
        let (_air, host, client) = pair();
        client.mp_clock(4_900);
        host.mp_send_cmd(b"cmd", 5_000);

        let answering = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(5));
            client.mp_send_reply(b"hello", 5_010, 1);
        });

        let mut buf = vec![0u8; 15 * 1024];
        let mask = host.mp_recv_replies(&mut buf, 0, 5_000, 0b10);
        answering.join().unwrap();

        assert_eq!(mask, 0b10, "AID 1 answered, late but within the round's wait");
        assert_eq!(&buf[..5], b"hello");
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
