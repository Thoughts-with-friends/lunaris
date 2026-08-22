//! One console's view of the airwaves, and the `melonds::Host` it presents.

use super::*;

/// One console's view of the airwaves.
#[derive(Clone)]
pub struct Client {
    pub(crate) airwaves: Airwaves,
    pub(crate) instance: usize,
}

impl Client {
    pub(crate) fn begin(&self) {
        let mut shared = self.airwaves.0.0.lock().unwrap();
        shared.boxes[self.instance].connected = true;
    }

    pub(crate) fn end(&self) {
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
    pub(crate) fn send(&self, kind: Kind, data: &[u8], timestamp: u64) -> i32 {
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
    pub(crate) fn peer_is_live(shared: &Shared, me: usize) -> bool {
        shared.boxes.iter().enumerate().any(|(i, mailbox)| {
            i != me
                && mailbox.connected
                && mailbox.active.is_some_and(|at| at.elapsed() < PEER_TIMEOUT)
        })
    }

    /// Take the next ordinary packet, if one is waiting. `now` is the
    /// receiving console's own wifi clock, for the trace only.
    pub(crate) fn recv(&self, out: &mut [u8], now: u64, timestamp: &mut u64) -> i32 {
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
    pub(crate) fn wait_for_traffic(&self, deadline: Instant) -> bool {
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
    pub(crate) fn collect_replies(
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

/// How far behind the host's clock a reply may be and still count. melonDS uses
/// the same 32 microseconds in `RecvReplies`.
pub(crate) const STALE_MICROSECONDS: u64 = 32;

/// How long a blocking receive waits, in *wall* time. melonDS's
/// `MPInterface::RecvTimeout` is 25 ms and its receives wait on a semaphore for
/// that long; this waits on a condvar instead, for the same reason and the same
/// duration — the peer is another thread now, and the answer is expected while
/// we wait.
pub(crate) const RECV_TIMEOUT: Duration = Duration::from_millis(25);

/// How stale a peer's last activity may be before a blocking receive stops
/// waiting for it.
///
/// A console that is paused, stopped, or still booting cannot answer, and
/// waiting out the full timeout on every round for one that never will is what
/// turns "the second window is paused" into a front end running at two frames a
/// second. Generous enough that a console merely running slowly still counts.
pub(crate) const PEER_TIMEOUT: Duration = Duration::from_millis(250);
