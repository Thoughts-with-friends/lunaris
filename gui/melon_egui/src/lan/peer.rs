//! One end of a link: the socket, the inboxes, and the threads that feed them.

use super::*;

// -- the peer ----------------------------------------------------------------

/// One end of a link: a socket, the two inboxes, and everything measured about
/// it.
pub(crate) struct Peer {
    pub(crate) socket: UdpSocket,
    pub(crate) remote: SocketAddr,
    /// Beacons, association, CMD and ACK — everything the core reads with
    /// `mp_recv_packet` or `mp_recv_host_packet`.
    pub(crate) regular: Queue,
    /// Clients' answers, which the host drains all at once per round.
    pub(crate) replies: Queue,
    pub(crate) measurements: Measurements,
    pub(crate) pace: LinkPace,
    pub(crate) tuning: Tuning,
    /// Next datagram sequence number. Wraps, which the window in `seen`
    /// tolerates because it only ever compares for equality.
    pub(crate) sequence: AtomicU32,
    /// Sequence numbers already accepted, newest last.
    pub(crate) seen: Mutex<VecDeque<u32>>,
    pub(crate) pending: Mutex<Coalescer>,
    pub(crate) shutdown: Arc<AtomicBool>,
    /// Set once `mp_begin` has been called, so the flusher does not send while
    /// the console's wireless is off.
    pub(crate) live: AtomicBool,
}

impl Peer {
    pub(crate) fn start(
        socket: UdpSocket,
        remote: SocketAddr,
        mut tuning: Tuning,
    ) -> io::Result<(Arc<Self>, LinkPace)> {
        tuning.normalize();
        // Short enough that shutdown is prompt, long enough that an idle link
        // is not a spin loop.
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
        let pace = LinkPace::default();
        let peer = Arc::new(Self {
            socket,
            remote,
            regular: Queue::default(),
            replies: Queue::default(),
            measurements: Measurements::default(),
            pace: pace.clone(),
            tuning,
            sequence: AtomicU32::new(1),
            seen: Mutex::new(VecDeque::with_capacity(SEEN_WINDOW)),
            pending: Mutex::new(Coalescer::default()),
            shutdown: Arc::new(AtomicBool::new(false)),
            live: AtomicBool::new(false),
        });

        for (name, body) in [
            ("melon_egui-lan-rx", Arc::clone(&peer) as Arc<Peer>),
            ("melon_egui-lan-tx", Arc::clone(&peer)),
        ] {
            let receive = name.ends_with("-rx");
            std::thread::Builder::new()
                .name(name.to_owned())
                .spawn(move || {
                    if receive {
                        body.receive_loop();
                    } else {
                        body.service_loop();
                    }
                })
                .map_err(|error| io::Error::other(format!("cannot start {name}: {error}")))?;
        }
        Ok((peer, pace))
    }

    /// The wait a round is allowed, from what the link has been measured doing.
    ///
    /// A reply cannot come back faster than one round trip, so that is the
    /// floor; the jitter term is what covers the *variance*, which on a VPN is
    /// usually the larger problem. Both ends are clamped by [`Tuning`].
    pub(crate) fn budget(&self) -> Duration {
        let rtt = self.measurements.rtt_us.load(Ordering::Relaxed);
        let jitter = self.measurements.jitter_us.load(Ordering::Relaxed);
        let estimate = rtt + jitter * u64::from(self.tuning.jitter_factor);
        let floor = u64::from(self.tuning.min_budget_ms) * 1000;
        let ceiling = u64::from(self.tuning.max_budget_ms) * 1000;
        Duration::from_micros(estimate.clamp(floor, ceiling))
    }

    /// How far back in emulated time a reply may be stamped and still count.
    ///
    /// melonDS's own local transport uses a flat 32 µs because both consoles
    /// are on one machine and a reply is *expected* within a round. Over a link
    /// the reply is genuinely older than that by the time it lands, so the
    /// window is the wire delay expressed in emulated microseconds — the two
    /// clocks run at the same rate, which is what makes the conversion a
    /// no-op — plus a margin for the frame the reply was produced in.
    pub(crate) fn stale_window_us(&self) -> u64 {
        // One emulated frame is 16 716 µs; a reply from the previous frame is
        // still the answer to this round on a link this slow.
        self.budget().as_micros().min(u128::from(u64::MAX)) as u64 + 16_716
    }

    /// Whether this datagram has been seen before, recording it if not.
    pub(crate) fn is_duplicate(&self, sequence: u32) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if seen.contains(&sequence) {
            return true;
        }
        if seen.len() >= SEEN_WINDOW {
            seen.pop_front();
        }
        seen.push_back(sequence);
        false
    }

    pub(crate) fn receive_loop(&self) {
        let mut buffer = vec![0u8; MAX_DATAGRAM.max(HEADER_LEN + MAX_PAYLOAD) * 2];
        while !self.shutdown.load(Ordering::Relaxed) {
            let Ok((len, sender)) = self.socket.recv_from(&mut buffer) else {
                continue;
            };
            if sender != self.remote {
                continue;
            }
            let Some((sequence, frames)) = decode(&buffer[..len]) else {
                continue;
            };
            self.measurements.datagrams_received.fetch_add(1, Ordering::Relaxed);
            // Sequence 0 is the handshake, which predates the counter and may
            // legitimately repeat.
            if sequence != 0 && self.is_duplicate(sequence) {
                self.measurements.duplicates_dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            for frame in frames {
                self.measurements.frames_received.fetch_add(1, Ordering::Relaxed);
                match frame.kind {
                    // Answered on this thread rather than queued: the whole
                    // point of the probe is to time the path, and time spent
                    // waiting for the emulation thread to notice is not part of
                    // the path.
                    Kind::Ping => {
                        self.transmit(&self.one(Kind::Pong, 0, frame.timestamp, &[]), 1);
                    }
                    Kind::Pong => {
                        let sent = Duration::from_micros(frame.timestamp);
                        if let Some(rtt) = wall_clock_micros().checked_sub(frame.timestamp) {
                            let _ = sent;
                            self.measurements.observe_rtt(Duration::from_micros(rtt));
                        }
                    }
                    Kind::Reply => self.replies.push(frame),
                    Kind::Hello | Kind::Welcome => {}
                    _ => self.regular.push(frame),
                }
            }
        }
    }

    /// Flush expired batches and send the periodic latency probe.
    ///
    /// Both live on a thread of their own so that neither depends on the
    /// console reaching a `Host` callback: a console that is paused, or between
    /// carts, must still answer probes, or the peer's budget decays towards its
    /// ceiling for no reason.
    pub(crate) fn service_loop(&self) {
        let window = Duration::from_millis(u64::from(self.tuning.batch_window_ms));
        let mut next_ping = Instant::now();
        while !self.shutdown.load(Ordering::Relaxed) {
            if !window.is_zero() {
                let expired = {
                    let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
                    pending.expired(window).then(|| pending.take()).flatten()
                };
                if let Some((bytes, _)) = expired {
                    self.transmit(&bytes, 1);
                }
            }
            if Instant::now() >= next_ping {
                self.transmit(&self.one(Kind::Ping, 0, wall_clock_micros(), &[]), 1);
                next_ping += PING_INTERVAL;
            }
            // Fine enough to honour a 3 ms batch window without becoming a spin
            // loop of its own.
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// One frame, encoded on its own.
    pub(crate) fn one(&self, kind: Kind, aid: u16, timestamp: u64, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
        encode_into(&mut bytes, kind, aid, timestamp, payload);
        bytes
    }

    /// Put one datagram on the wire `copies` times, stamped with a fresh
    /// sequence so the peer can tell the copies from a genuine repeat.
    ///
    /// Returns whether at least one copy left the socket.
    pub(crate) fn transmit(&self, bytes: &[u8], copies: u8) -> bool {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let mut stamped = bytes.to_vec();
        stamp_sequence(&mut stamped, sequence);
        let mut sent = false;
        for _ in 0..copies.max(1) {
            if self.socket.send_to(&stamped, self.remote).is_ok() {
                sent = true;
                self.measurements.datagrams_sent.fetch_add(1, Ordering::Relaxed);
            }
        }
        sent
    }

    /// Send one wireless frame, batching it if its kind allows.
    ///
    /// Returns what the core's platform callbacks expect: the payload length on
    /// success, or -1 if the frame could not be sent at all.
    pub(crate) fn send(&self, kind: Kind, payload: &[u8], timestamp: u64, aid: u16) -> i32 {
        if payload.len() > MAX_PAYLOAD {
            log::warn!("lan: refusing a {}-byte frame (max {MAX_PAYLOAD})", payload.len());
            return -1;
        }
        self.measurements.frames_sent.fetch_add(1, Ordering::Relaxed);

        let window = Duration::from_millis(u64::from(self.tuning.batch_window_ms));
        if kind.is_urgent() || window.is_zero() {
            // Anything still batched has to precede this on the wire: the peer
            // reads a datagram's frames in order, but two datagrams can be
            // reordered, and a CMD that overtakes the beacon before it is a
            // round the client has no context for.
            let held = self.pending.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some((bytes, _)) = held {
                self.transmit(&bytes, 1);
            }
            let copies =
                if matches!(kind, Kind::Cmd | Kind::Reply) { self.tuning.reply_copies } else { 1 };
            let bytes = self.one(kind, aid, timestamp, payload);
            return if self.transmit(&bytes, copies) { payload.len() as i32 } else { -1 };
        }

        // Ordinary frame: hold it, and flush now only if this one would take
        // the datagram past the MTU-safe size.
        let flush = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            let full = pending.bytes.len() + HEADER_LEN + payload.len() > MAX_DATAGRAM;
            let ready = full.then(|| pending.take()).flatten();
            encode_into(&mut pending.bytes, kind, aid, timestamp, payload);
            pending.count += 1;
            pending.since.get_or_insert_with(Instant::now);
            ready
        };
        if let Some((bytes, _)) = flush {
            self.transmit(&bytes, 1);
        }
        payload.len() as i32
    }

    /// Take one ordinary frame without waiting, as melonDS's `RecvPacket`
    /// does: this is polled from the emulated wifi's idle path, and a wait here
    /// would be a wait in every frame whether or not a peer is talking.
    pub(crate) fn recv_packet(&self, data: &mut [u8], timestamp: &mut u64) -> Option<i32> {
        receive(&self.regular, data, timestamp, None, |frame| frame.kind == Kind::Packet)
    }

    /// Wait — for as long as the link has been measured to need — for the frame
    /// a client is expecting from its host.
    ///
    /// This is the other call that a fixed 25 ms ceiling breaks: past that
    /// round trip the client never sees its host's CMD at all.
    pub(crate) fn recv_host_packet(&self, data: &mut [u8], timestamp: &mut u64) -> Option<i32> {
        receive(&self.regular, data, timestamp, Some(self.budget()), |frame| {
            matches!(frame.kind, Kind::Packet | Kind::Cmd | Kind::Ack)
        })
    }

    /// Collect this round's replies, waiting up to the measured budget for the
    /// clients named in `aidmask`.
    ///
    /// Returns as soon as every addressed client has answered, so a healthy
    /// link costs one round trip rather than the whole budget — which is what
    /// keeps the frame rate up when the link is fine and spends the budget only
    /// when it is not.
    pub(crate) fn recv_replies(&self, data: &mut [u8], timestamp: u64, aidmask: u16) -> u16 {
        let started = Instant::now();
        let deadline = started + self.budget();
        let stale_window = self.stale_window_us();
        let mut answered = 0u16;

        while answered & aidmask != aidmask {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Some(frame) = self.replies.pop(Some(remaining), |frame| frame.kind == Kind::Reply)
            else {
                break;
            };
            // A reply stamped before this round began by more than the link's
            // own delay is answering an earlier round.
            if frame.timestamp.saturating_add(stale_window) < timestamp {
                self.measurements.stale_replies.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            if frame.aid == 0 || frame.aid >= 16 || aidmask & (1 << frame.aid) == 0 {
                continue;
            }
            // Reply slots are 1024 bytes each, indexed by AID-1 — melonDS's
            // `RecvReplies` layout, which the core reads back by the same
            // arithmetic.
            let offset = (frame.aid as usize - 1) * 1024;
            if offset < data.len() {
                let end = (offset + frame.payload.len()).min(data.len());
                data[offset..end].copy_from_slice(&frame.payload[..end - offset]);
            }
            answered |= 1 << frame.aid;
        }

        let waited = started.elapsed();
        if answered & aidmask == aidmask {
            self.measurements.rounds_answered.fetch_add(1, Ordering::Relaxed);
        } else {
            self.measurements.rounds_timed_out.fetch_add(1, Ordering::Relaxed);
        }
        self.measurements.observe_round_wait(waited);
        if self.tuning.pace_to_link {
            self.pace.observe(waited);
        }
        answered
    }

    /// Wireless is on: the console has called `MP_Begin`.
    pub(crate) fn begin(&self) {
        self.live.store(true, Ordering::Relaxed);
    }

    /// Wireless is off. Whatever is queued answers a session that is over, and
    /// delivering it to the next one would look like traffic from a peer that
    /// is not there.
    pub(crate) fn end(&self) {
        self.live.store(false, Ordering::Relaxed);
        self.regular.clear();
        self.replies.clear();
    }

    pub(crate) fn stats(&self) -> LinkStats {
        let m = &self.measurements;
        LinkStats {
            rtt_ms: m.rtt_us.load(Ordering::Relaxed) as f32 / 1000.0,
            jitter_ms: m.jitter_us.load(Ordering::Relaxed) as f32 / 1000.0,
            budget_ms: self.budget().as_secs_f32() * 1000.0,
            datagrams_sent: m.datagrams_sent.load(Ordering::Relaxed),
            datagrams_received: m.datagrams_received.load(Ordering::Relaxed),
            frames_sent: m.frames_sent.load(Ordering::Relaxed),
            frames_received: m.frames_received.load(Ordering::Relaxed),
            duplicates_dropped: m.duplicates_dropped.load(Ordering::Relaxed),
            rounds_answered: m.rounds_answered.load(Ordering::Relaxed),
            rounds_timed_out: m.rounds_timed_out.load(Ordering::Relaxed),
            stale_replies: m.stale_replies.load(Ordering::Relaxed),
            sustainable_fps: self.pace.frame_rate() as f32,
            wireless_on: self.live.load(Ordering::Relaxed),
        }
    }
}

/// The sender's wall clock in microseconds, for the latency probe.
///
/// Wall clock rather than [`Instant`] because the value crosses the wire; only
/// the *difference* between two readings on **the same machine** is ever used
/// (a `Ping` is timed by the host that sent it, from its own echo), so the two
/// consoles' clocks do not need to agree.
pub(crate) fn wall_clock_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_micros().min(u128::from(u64::MAX)) as u64)
}

/// Copy the first matching frame into the core's buffer.
pub(crate) fn receive<F>(
    queue: &Queue,
    data: &mut [u8],
    timestamp: &mut u64,
    wait: Option<Duration>,
    wanted: F,
) -> Option<i32>
where
    F: Fn(&Frame) -> bool,
{
    let frame = queue.pop(wait, wanted)?;
    let end = frame.payload.len().min(data.len());
    data[..end].copy_from_slice(&frame.payload[..end]);
    *timestamp = frame.timestamp;
    Some(end as i32)
}
