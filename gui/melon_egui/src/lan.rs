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
enum Kind {
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
struct Frame {
    kind: Kind,
    aid: u16,
    /// The sender's emulated wifi clock, in microseconds — except on
    /// `Ping`/`Pong`, where it is the sender's wall clock.
    timestamp: u64,
    payload: Vec<u8>,
}

// -- tuning -----------------------------------------------------------------

/// The knobs a user can turn, persisted in the instance's `settings.json`.
///
/// Defaults are the LAN-friendly end of each range: a link that needs none of
/// this behaves as `melonds::lan` did, only without the fixed 25 ms ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Tuning {
    /// Floor for the reply wait, in milliseconds. Never below this even on a
    /// link that measures faster, so a single fast probe cannot starve a round.
    pub min_budget_ms: u16,
    /// Ceiling for the reply wait, in milliseconds. This is the real limit on
    /// how bad a link the transport will still try to play over: past it the
    /// emulated frame rate is so low the game is unplayable anyway.
    pub max_budget_ms: u16,
    /// How many multiples of the measured jitter are added to the budget on top
    /// of the round trip. Higher rides out a spikier link at the cost of
    /// waiting longer on the frames that *are* lost.
    pub jitter_factor: u8,
    /// How many copies of each reply and CMD go on the wire. 1 disables
    /// redundancy; 2 is the default and removes the great majority of
    /// single-packet losses.
    pub reply_copies: u8,
    /// How long ordinary (non-round) frames may wait to share a datagram, in
    /// milliseconds. 0 disables batching.
    pub batch_window_ms: u16,
    /// Whether the emulated frame rate follows what the link can sustain. Off,
    /// the console runs at 59.83 Hz and drops rounds it cannot service.
    pub pace_to_link: bool,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            min_budget_ms: 8,
            // 250 ms covers a transcontinental VPN. Past that a DS game's own
            // timeouts give up regardless of what this does.
            max_budget_ms: 250,
            jitter_factor: 4,
            reply_copies: 2,
            // Off by default. Batching only ever applies to beacons and the
            // association handshake, and delaying those is the one thing here
            // that could break a link which currently works — while the gain,
            // fewer datagrams on a per-packet-costly tunnel, is unmeasured in
            // operation. It is offered as a knob rather than imposed.
            batch_window_ms: 0,
            pace_to_link: true,
        }
    }
}

impl Tuning {
    /// Clamp every field to a range the transport can actually honour, so a
    /// hand-edited `settings.json` cannot put the link in a state the UI could
    /// not produce.
    pub fn normalize(&mut self) {
        self.min_budget_ms = self.min_budget_ms.clamp(1, 200);
        self.max_budget_ms = self.max_budget_ms.clamp(self.min_budget_ms, 1000);
        self.jitter_factor = self.jitter_factor.clamp(0, 16);
        self.reply_copies = self.reply_copies.clamp(1, 4);
        self.batch_window_ms = self.batch_window_ms.min(50);
    }
}

// -- link measurement --------------------------------------------------------

/// What the link is doing, in numbers the UI can show and the transport can act
/// on.
///
/// Every field is atomic because it is written by the receive thread and the
/// emulation thread and read by the UI thread, none of which should ever wait
/// on another to draw a label.
#[derive(Default)]
struct Measurements {
    /// Smoothed round trip, in microseconds.
    rtt_us: AtomicU64,
    /// Smoothed absolute variation in round trip, in microseconds.
    jitter_us: AtomicU64,
    /// Datagrams sent and received, and frames within them.
    datagrams_sent: AtomicU64,
    datagrams_received: AtomicU64,
    frames_sent: AtomicU64,
    frames_received: AtomicU64,
    /// Datagrams discarded because their sequence number had been seen — the
    /// redundancy of [`Tuning::reply_copies`] doing its job.
    duplicates_dropped: AtomicU64,
    /// Rounds the host asked for and fully collected.
    rounds_answered: AtomicU64,
    /// Rounds that timed out with at least one client unheard. This is the
    /// number that becomes a communication error in the game.
    rounds_timed_out: AtomicU64,
    /// Replies discarded for arriving outside the staleness window.
    stale_replies: AtomicU64,
    /// Smoothed time the host actually spent waiting per round, in
    /// microseconds. What [`LinkPace`] is computed from.
    round_wait_us: AtomicU64,
}

/// A snapshot of [`Measurements`], for the diagnostics pane.
#[derive(Clone, Copy, Default, Debug)]
pub struct LinkStats {
    pub rtt_ms: f32,
    pub jitter_ms: f32,
    pub budget_ms: f32,
    pub datagrams_sent: u64,
    pub datagrams_received: u64,
    pub frames_sent: u64,
    pub frames_received: u64,
    pub duplicates_dropped: u64,
    pub rounds_answered: u64,
    pub rounds_timed_out: u64,
    pub stale_replies: u64,
    /// The frame rate the link can sustain, as [`LinkPace`] reports it.
    pub sustainable_fps: f32,
    /// Whether the console has switched its wireless on (`MP_Begin`). A link
    /// that is connected but not live is a cart that has not reached its
    /// multiplayer menu yet, which is worth being able to tell apart from a
    /// link that is silent because it is broken.
    pub wireless_on: bool,
}

impl LinkStats {
    /// The fraction of rounds that completed, or `None` before any round has
    /// been attempted.
    ///
    /// This is the single number that says whether the link is working: a game
    /// reports a communication error when this falls, and nothing else here
    /// predicts it as directly.
    #[must_use]
    pub fn round_success(&self) -> Option<f32> {
        let attempted = self.rounds_answered + self.rounds_timed_out;
        (attempted > 0).then(|| self.rounds_answered as f32 / attempted as f32)
    }
}

/// The emulated frame rate the link can sustain, shared with the front end.
///
/// Stored as millihertz in an atomic so the UI thread can read it without a
/// lock, and so the emulation thread can write it from inside a `Host`
/// callback — which is to say from inside `run_frame`, where taking a lock the
/// UI thread might hold would deadlock the console.
#[derive(Clone, Default)]
pub struct LinkPace(Arc<AtomicU32>);

/// The DS's own video frame rate, `33_513_982 / 560_190` Hz.
const NATIVE_FPS: f64 = 59.826_098;

impl LinkPace {
    /// The frame rate to pace at, never above the DS's own and never so low the
    /// window feels dead.
    ///
    /// `NATIVE_FPS` until the first round has been timed, which is the right
    /// answer for a console that has not started talking yet.
    #[must_use]
    pub fn frame_rate(&self) -> f64 {
        match self.0.load(Ordering::Relaxed) {
            0 => NATIVE_FPS,
            millihertz => f64::from(millihertz) / 1000.0,
        }
    }

    /// Record how long a round took and update the sustainable rate.
    ///
    /// The console cannot issue frames faster than one per
    /// `native frame time + the wait a round costs`, because the wait happens
    /// *inside* the frame. Pacing to anything faster only builds a debt that is
    /// discharged as a burst of rounds, which is worse for the peer than
    /// running slow.
    fn observe(&self, round_wait: Duration) {
        let period = 1.0 / NATIVE_FPS + round_wait.as_secs_f64();
        let fps = (1.0 / period).clamp(5.0, NATIVE_FPS);
        // A round that completes instantly must not snap the rate back to
        // native while the next one is about to block again, so the estimate is
        // smoothed the same way the round trip is.
        let previous = self.frame_rate();
        let smoothed = previous.mul_add(0.875, fps * 0.125);
        self.0.store((smoothed * 1000.0) as u32, Ordering::Relaxed);
    }
}

impl Measurements {
    /// Fold one round-trip sample into the smoothed estimates.
    ///
    /// The same exponentially weighted estimator RFC 6298 §2.3 specifies for
    /// TCP, with its 1/8 and 1/4 gains: it is well understood, cheap, and
    /// reacts to a VPN's step change in latency within about a second at
    /// [`PING_INTERVAL`].
    fn observe_rtt(&self, sample: Duration) {
        let sample = sample.as_micros().min(u128::from(u64::MAX)) as u64;
        let previous = self.rtt_us.load(Ordering::Relaxed);
        if previous == 0 {
            self.rtt_us.store(sample, Ordering::Relaxed);
            self.jitter_us.store(sample / 2, Ordering::Relaxed);
            return;
        }
        let deviation = previous.abs_diff(sample);
        let jitter = self.jitter_us.load(Ordering::Relaxed);
        self.jitter_us.store((jitter * 3 + deviation) / 4, Ordering::Relaxed);
        self.rtt_us.store((previous * 7 + sample) / 8, Ordering::Relaxed);
    }

    fn observe_round_wait(&self, waited: Duration) {
        let sample = waited.as_micros().min(u128::from(u64::MAX)) as u64;
        let previous = self.round_wait_us.load(Ordering::Relaxed);
        let smoothed = if previous == 0 { sample } else { (previous * 7 + sample) / 8 };
        self.round_wait_us.store(smoothed, Ordering::Relaxed);
    }
}

// -- frame queues ------------------------------------------------------------

/// One side's inbox for a class of frames.
#[derive(Default)]
struct Queue {
    frames: Mutex<VecDeque<(Instant, Frame)>>,
    arrived: Condvar,
}

impl Queue {
    fn push(&self, frame: Frame) {
        let mut frames = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        if frames.len() >= QUEUE_CAPACITY {
            // Anything this old cannot answer a round that is still open, so it
            // goes first — that is the whole of the room-making, when there is
            // any to be had. `melonds::lan` skips this step and pops the front
            // unconditionally, which under a jitter burst throws away frames
            // that are still wanted while stale ones sit behind them.
            while frames.front().is_some_and(|(at, _)| at.elapsed() > STALE_FRAME_AGE) {
                frames.pop_front();
            }
            // Still full: everything queued is recent, so this is genuine
            // overload and the oldest is the least bad thing to lose.
            if frames.len() >= QUEUE_CAPACITY {
                frames.pop_front();
            }
        }
        frames.push_back((Instant::now(), frame));
        self.arrived.notify_all();
    }

    /// Take the first frame `wanted` accepts, waiting up to `timeout` for one.
    ///
    /// `timeout` of `None` does not wait at all, which is what the non-blocking
    /// `mp_recv_packet` needs.
    fn pop<F>(&self, timeout: Option<Duration>, wanted: F) -> Option<Frame>
    where
        F: Fn(&Frame) -> bool,
    {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut frames = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(at) = frames.iter().position(|(_, frame)| wanted(frame)) {
                return frames.remove(at).map(|(_, frame)| frame);
            }
            // No deadline means a non-blocking poll: the queue held nothing
            // wanted, and that is the answer.
            let remaining = deadline?.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let (guard, _) =
                self.arrived.wait_timeout(frames, remaining).unwrap_or_else(|e| e.into_inner());
            frames = guard;
        }
    }

    fn clear(&self) {
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

// -- wire format -------------------------------------------------------------

/// Append one frame to `bytes`. Several frames may share a datagram; the
/// decoder loops until the buffer is consumed, which is what makes
/// [`Coalescer`] possible without a second envelope layer.
fn encode_into(bytes: &mut Vec<u8>, kind: Kind, aid: u16, timestamp: u64, payload: &[u8]) {
    bytes.extend_from_slice(MAGIC);
    bytes.push(kind as u8);
    bytes.extend_from_slice(&aid.to_le_bytes());
    bytes.extend_from_slice(&timestamp.to_le_bytes());
    // Sequence is stamped per datagram, not per frame, and filled in by
    // `Peer::transmit`; a placeholder keeps the layout fixed-width.
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    bytes.extend_from_slice(payload);
}

/// Overwrite every frame header's sequence field in a finished datagram.
///
/// Every frame in one datagram shares its sequence number, because the
/// duplicate rejection this feeds is about datagrams: a redundant copy repeats
/// the whole datagram, not one frame of it.
fn stamp_sequence(bytes: &mut [u8], sequence: u32) {
    let mut at = 0;
    while at + HEADER_LEN <= bytes.len() {
        let len = u16::from_le_bytes([bytes[at + 19], bytes[at + 20]]) as usize;
        bytes[at + 15..at + 19].copy_from_slice(&sequence.to_le_bytes());
        at += HEADER_LEN + len;
    }
}

/// Read every frame out of one datagram, with the sequence they share.
///
/// Returns `None` for a datagram that is not ours or is malformed, which is not
/// an error worth reporting: a UDP port takes whatever is sent to it.
fn decode(bytes: &[u8]) -> Option<(u32, Vec<Frame>)> {
    let mut frames = Vec::new();
    let mut sequence = None;
    let mut at = 0;
    while at + HEADER_LEN <= bytes.len() {
        if &bytes[at..at + 4] != MAGIC {
            return None;
        }
        let kind = Kind::from_wire(bytes[at + 4])?;
        let aid = u16::from_le_bytes([bytes[at + 5], bytes[at + 6]]);
        let timestamp = u64::from_le_bytes(bytes[at + 7..at + 15].try_into().ok()?);
        let seq = u32::from_le_bytes(bytes[at + 15..at + 19].try_into().ok()?);
        let len = u16::from_le_bytes([bytes[at + 19], bytes[at + 20]]) as usize;
        if len > MAX_PAYLOAD || at + HEADER_LEN + len > bytes.len() {
            return None;
        }
        let payload = bytes[at + HEADER_LEN..at + HEADER_LEN + len].to_vec();
        sequence.get_or_insert(seq);
        frames.push(Frame { kind, aid, timestamp, payload });
        at += HEADER_LEN + len;
    }
    // A trailing partial frame means a truncated datagram; the frames before it
    // are still good, but a datagram with nothing in it is not.
    (!frames.is_empty()).then(|| (sequence.unwrap_or(0), frames))
}

// -- batching ----------------------------------------------------------------

/// Ordinary frames waiting to share a datagram.
///
/// Only `Kind::Packet` is ever held here — see the module documentation for why
/// a round's frames cannot be. Held frames go out when the buffer would exceed
/// [`MAX_DATAGRAM`], when a round frame overtakes them (so the peer never sees a
/// CMD before the beacon that preceded it), or when
/// [`Tuning::batch_window_ms`] elapses, whichever comes first.
#[derive(Default)]
struct Coalescer {
    bytes: Vec<u8>,
    /// When the oldest held frame was buffered.
    since: Option<Instant>,
    count: u32,
}

impl Coalescer {
    /// Whether the buffer has been waiting longer than the window allows.
    fn expired(&self, window: Duration) -> bool {
        self.since.is_some_and(|since| since.elapsed() >= window)
    }

    fn take(&mut self) -> Option<(Vec<u8>, u32)> {
        if self.bytes.is_empty() {
            return None;
        }
        self.since = None;
        let count = std::mem::take(&mut self.count);
        Some((std::mem::take(&mut self.bytes), count))
    }
}

// -- the peer ----------------------------------------------------------------

/// One end of a link: a socket, the two inboxes, and everything measured about
/// it.
struct Peer {
    socket: UdpSocket,
    remote: SocketAddr,
    /// Beacons, association, CMD and ACK — everything the core reads with
    /// `mp_recv_packet` or `mp_recv_host_packet`.
    regular: Queue,
    /// Clients' answers, which the host drains all at once per round.
    replies: Queue,
    measurements: Measurements,
    pace: LinkPace,
    tuning: Tuning,
    /// Next datagram sequence number. Wraps, which the window in `seen`
    /// tolerates because it only ever compares for equality.
    sequence: AtomicU32,
    /// Sequence numbers already accepted, newest last.
    seen: Mutex<VecDeque<u32>>,
    pending: Mutex<Coalescer>,
    shutdown: Arc<AtomicBool>,
    /// Set once `mp_begin` has been called, so the flusher does not send while
    /// the console's wireless is off.
    live: AtomicBool,
}

impl Peer {
    fn start(
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
    fn budget(&self) -> Duration {
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
    fn stale_window_us(&self) -> u64 {
        // One emulated frame is 16 716 µs; a reply from the previous frame is
        // still the answer to this round on a link this slow.
        self.budget().as_micros().min(u128::from(u64::MAX)) as u64 + 16_716
    }

    /// Whether this datagram has been seen before, recording it if not.
    fn is_duplicate(&self, sequence: u32) -> bool {
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

    fn receive_loop(&self) {
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
    fn service_loop(&self) {
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
    fn one(&self, kind: Kind, aid: u16, timestamp: u64, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
        encode_into(&mut bytes, kind, aid, timestamp, payload);
        bytes
    }

    /// Put one datagram on the wire `copies` times, stamped with a fresh
    /// sequence so the peer can tell the copies from a genuine repeat.
    ///
    /// Returns whether at least one copy left the socket.
    fn transmit(&self, bytes: &[u8], copies: u8) -> bool {
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
    fn send(&self, kind: Kind, payload: &[u8], timestamp: u64, aid: u16) -> i32 {
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
    fn recv_packet(&self, data: &mut [u8], timestamp: &mut u64) -> Option<i32> {
        receive(&self.regular, data, timestamp, None, |frame| frame.kind == Kind::Packet)
    }

    /// Wait — for as long as the link has been measured to need — for the frame
    /// a client is expecting from its host.
    ///
    /// This is the other call that a fixed 25 ms ceiling breaks: past that
    /// round trip the client never sees its host's CMD at all.
    fn recv_host_packet(&self, data: &mut [u8], timestamp: &mut u64) -> Option<i32> {
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
    fn recv_replies(&self, data: &mut [u8], timestamp: u64, aidmask: u16) -> u16 {
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
    fn begin(&self) {
        self.live.store(true, Ordering::Relaxed);
    }

    /// Wireless is off. Whatever is queued answers a session that is over, and
    /// delivering it to the next one would look like traffic from a peer that
    /// is not there.
    fn end(&self) {
        self.live.store(false, Ordering::Relaxed);
        self.regular.clear();
        self.replies.clear();
    }

    fn stats(&self) -> LinkStats {
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
fn wall_clock_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_micros().min(u128::from(u64::MAX)) as u64)
}

// -- the two ends ------------------------------------------------------------

/// The host side of a link: binds a port and waits for one guest.
pub struct LanHost {
    peer: Arc<Peer>,
    pace: LinkPace,
}

/// The guest side of a link: connects to a host and waits for its welcome.
pub struct LanGuest {
    peer: Arc<Peer>,
    pace: LinkPace,
}

impl LanHost {
    /// Bind `bind_addr` and wait for a guest's `HELLO`, answering it.
    ///
    /// Blocks until one arrives, so the caller runs it off the UI thread.
    ///
    /// # Errors
    ///
    /// If the port cannot be bound, or the socket fails while waiting.
    pub fn accept(bind_addr: SocketAddr, tuning: Tuning) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        let mut buffer = vec![0u8; HEADER_LEN + MAX_PAYLOAD];
        loop {
            match socket.recv_from(&mut buffer) {
                Ok((len, guest)) => {
                    let Some((_, frames)) = decode(&buffer[..len]) else { continue };
                    if !frames.iter().any(|frame| frame.kind == Kind::Hello) {
                        continue;
                    }
                    let mut welcome = Vec::new();
                    encode_into(&mut welcome, Kind::Welcome, 0, 0, &[]);
                    // Sent before the peer exists, so it is not `transmit`'s
                    // business; three copies because losing the welcome costs
                    // the guest its whole connection attempt.
                    for _ in 0..3 {
                        socket.send_to(&welcome, guest)?;
                    }
                    let (peer, pace) = Peer::start(socket, guest, tuning)?;
                    return Ok(Self { peer, pace });
                }
                Err(ref error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// The address this end is bound to.
    ///
    /// # Errors
    /// If the socket cannot report it.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.peer.socket.local_addr()
    }

    /// The guest that connected.
    #[must_use]
    pub fn remote_addr(&self) -> SocketAddr {
        self.peer.remote
    }

    /// What the link is doing, for the diagnostics pane.
    #[must_use]
    pub fn stats(&self) -> LinkStats {
        self.peer.stats()
    }

    /// The frame rate handle the front end paces the console to.
    #[must_use]
    pub fn pace(&self) -> LinkPace {
        self.pace.clone()
    }
}

impl LanGuest {
    /// Bind `bind_addr`, announce to `host_addr`, and wait for its welcome.
    ///
    /// Retries the announcement, because on a VPN the first datagram after the
    /// tunnel comes up is the one most likely to be dropped.
    ///
    /// # Errors
    ///
    /// If the port cannot be bound, or no welcome arrives.
    pub fn connect(
        bind_addr: SocketAddr,
        host_addr: SocketAddr,
        tuning: Tuning,
    ) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_read_timeout(Some(Duration::from_secs(1)))?;
        let mut hello = Vec::new();
        encode_into(&mut hello, Kind::Hello, 0, 0, &[]);
        let mut buffer = vec![0u8; HEADER_LEN + MAX_PAYLOAD];
        // Ten seconds in total. A tunnel that is still negotiating routinely
        // eats the first second or two, and failing inside that window makes
        // the front end look broken when it is merely early.
        for _ in 0..10 {
            socket.send_to(&hello, host_addr)?;
            match socket.recv_from(&mut buffer) {
                Ok((len, sender)) if sender == host_addr => {
                    let Some((_, frames)) = decode(&buffer[..len]) else { continue };
                    if frames.iter().any(|frame| frame.kind == Kind::Welcome) {
                        let (peer, pace) = Peer::start(socket, host_addr, tuning)?;
                        return Ok(Self { peer, pace });
                    }
                }
                Ok(_) => continue,
                Err(ref error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("no answer from {host_addr} after 10 attempts"),
        ))
    }

    /// The address this end is bound to.
    ///
    /// # Errors
    /// If the socket cannot report it.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.peer.socket.local_addr()
    }

    /// What the link is doing, for the diagnostics pane.
    #[must_use]
    pub fn stats(&self) -> LinkStats {
        self.peer.stats()
    }

    /// The frame rate handle the front end paces the console to.
    #[must_use]
    pub fn pace(&self) -> LinkPace {
        self.pace.clone()
    }
}

/// The `Host` half, identical for both ends: which side of the handshake a
/// console was on does not change how its wireless behaves.
///
/// Every method is a one-line forward to [`Peer`], where the behaviour lives.
/// That split is deliberate: the trait is only available when the `melonds`
/// feature links the core, and the transport's own tests — which are the
/// evidence that any of this helps — must be runnable without it.
#[cfg(feature = "melonds")]
macro_rules! impl_host {
    ($type:ty) => {
        impl melonds::Host for $type {
            fn mp_begin(&self) {
                self.peer.begin();
            }

            fn mp_end(&self) {
                self.peer.end();
            }

            fn mp_send_packet(&self, data: &[u8], timestamp: u64) -> i32 {
                self.peer.send(Kind::Packet, data, timestamp, 0)
            }

            fn mp_send_cmd(&self, data: &[u8], timestamp: u64) -> i32 {
                self.peer.send(Kind::Cmd, data, timestamp, 0)
            }

            fn mp_send_reply(&self, data: &[u8], timestamp: u64, aid: u16) -> i32 {
                self.peer.send(Kind::Reply, data, timestamp, aid)
            }

            fn mp_send_ack(&self, data: &[u8], timestamp: u64) -> i32 {
                self.peer.send(Kind::Ack, data, timestamp, 0)
            }

            fn mp_recv_packet(
                &self,
                data: &mut [u8],
                _now: u64,
                timestamp: &mut u64,
            ) -> Option<i32> {
                self.peer.recv_packet(data, timestamp)
            }

            fn mp_recv_host_packet(
                &self,
                data: &mut [u8],
                _now: u64,
                timestamp: &mut u64,
            ) -> Option<i32> {
                self.peer.recv_host_packet(data, timestamp)
            }

            fn mp_recv_replies(
                &self,
                data: &mut [u8],
                _now: u64,
                timestamp: u64,
                aidmask: u16,
            ) -> u16 {
                self.peer.recv_replies(data, timestamp, aidmask)
            }
        }
    };
}

#[cfg(feature = "melonds")]
impl_host!(LanHost);
#[cfg(feature = "melonds")]
impl_host!(LanGuest);

/// Winding the receive and service threads up is the same on both ends, and has
/// to happen however the link ends — including when a connection attempt is
/// dropped half-built.
macro_rules! impl_drop {
    ($type:ty) => {
        impl Drop for $type {
            fn drop(&mut self) {
                self.peer.shutdown.store(true, Ordering::Relaxed);
            }
        }
    };
}

impl_drop!(LanHost);
impl_drop!(LanGuest);

/// Copy the first matching frame into the core's buffer.
fn receive<F>(
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        Frame, HEADER_LEN, Kind, LinkPace, MAX_DATAGRAM, Measurements, NATIVE_FPS, Queue, Tuning,
        decode, encode_into, stamp_sequence,
    };

    fn datagram(frames: &[(Kind, u16, u64, &[u8])], sequence: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (kind, aid, timestamp, payload) in frames {
            encode_into(&mut bytes, *kind, *aid, *timestamp, payload);
        }
        stamp_sequence(&mut bytes, sequence);
        bytes
    }

    #[test]
    fn a_single_frame_round_trips() {
        let bytes = datagram(&[(Kind::Reply, 3, 0x1234_5678, b"hello")], 9);
        let (sequence, frames) = decode(&bytes).expect("a valid datagram");
        assert_eq!(sequence, 9);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].kind, Kind::Reply);
        assert_eq!(frames[0].aid, 3);
        assert_eq!(frames[0].timestamp, 0x1234_5678);
        assert_eq!(frames[0].payload, b"hello");
    }

    /// The batching in [`super::Coalescer`] rests on this: several frames in
    /// one datagram must come back out in the order they went in, all carrying
    /// the datagram's sequence.
    #[test]
    fn batched_frames_round_trip_in_order() {
        let bytes = datagram(
            &[
                (Kind::Packet, 0, 100, b"beacon"),
                (Kind::Packet, 0, 200, b"probe-request"),
                (Kind::Packet, 0, 300, &[]),
            ],
            42,
        );
        let (sequence, frames) = decode(&bytes).expect("a valid datagram");
        assert_eq!(sequence, 42);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].payload, b"beacon");
        assert_eq!(frames[1].payload, b"probe-request");
        assert_eq!(frames[2].timestamp, 300);
        assert!(frames[2].payload.is_empty());
    }

    #[test]
    fn a_truncated_or_foreign_datagram_is_rejected() {
        let bytes = datagram(&[(Kind::Cmd, 0, 1, b"round")], 1);
        assert!(decode(&bytes[..bytes.len() - 2]).is_none());
        assert!(decode(b"not ours at all").is_none());
        let mut wrong_magic = bytes;
        wrong_magic[0] = b'X';
        assert!(decode(&wrong_magic).is_none());
    }

    /// Batching must not produce a datagram that IP would fragment: losing one
    /// fragment loses every frame in it, which is worse than sending two.
    #[test]
    fn a_full_batch_stays_under_the_mtu_ceiling() {
        let payload = [0u8; 200];
        let mut bytes = Vec::new();
        let mut count = 0;
        while bytes.len() + HEADER_LEN + payload.len() <= MAX_DATAGRAM {
            encode_into(&mut bytes, Kind::Packet, 0, 0, &payload);
            count += 1;
        }
        assert!(count > 1, "the ceiling must fit more than one frame or batching is pointless");
        assert!(bytes.len() <= MAX_DATAGRAM);
        assert_eq!(decode(&bytes).expect("a valid datagram").1.len(), count);
    }

    /// The defect this transport exists to fix: a fixed 25 ms budget cannot
    /// cover a link whose round trip is longer than that.
    #[test]
    fn the_budget_follows_the_measured_round_trip() {
        let tuning = Tuning::default();
        let measurements = Measurements::default();
        for _ in 0..40 {
            measurements.observe_rtt(Duration::from_millis(80));
        }
        let rtt = measurements.rtt_us.load(std::sync::atomic::Ordering::Relaxed);
        let jitter = measurements.jitter_us.load(std::sync::atomic::Ordering::Relaxed);
        let budget = (rtt + jitter * u64::from(tuning.jitter_factor))
            .clamp(u64::from(tuning.min_budget_ms) * 1000, u64::from(tuning.max_budget_ms) * 1000);
        assert!(
            budget >= 75_000,
            "an 80 ms link must get at least ~80 ms of budget, got {budget} us"
        );
        assert!(budget <= u64::from(tuning.max_budget_ms) * 1000);
    }

    #[test]
    fn a_lan_keeps_a_short_budget() {
        let tuning = Tuning::default();
        let measurements = Measurements::default();
        for _ in 0..40 {
            measurements.observe_rtt(Duration::from_micros(300));
        }
        let rtt = measurements.rtt_us.load(std::sync::atomic::Ordering::Relaxed);
        let jitter = measurements.jitter_us.load(std::sync::atomic::Ordering::Relaxed);
        let budget = (rtt + jitter * u64::from(tuning.jitter_factor))
            .max(u64::from(tuning.min_budget_ms) * 1000);
        assert_eq!(budget, u64::from(tuning.min_budget_ms) * 1000);
    }

    /// A round that blocks for 40 ms cannot be issued 60 times a second; the
    /// pace has to fall to what the link affords, or the front end builds a
    /// frame debt it discharges as a burst.
    #[test]
    fn the_pace_falls_to_what_the_link_affords() {
        let pace = LinkPace::default();
        assert!((pace.frame_rate() - NATIVE_FPS).abs() < 0.001);
        for _ in 0..200 {
            pace.observe(Duration::from_millis(40));
        }
        let rate = pace.frame_rate();
        assert!(rate < 30.0, "a 40 ms round must pace below 30 fps, got {rate}");
        assert!(rate > 15.0, "and not collapse: 1/(16.7ms + 40ms) is about 17.6 fps, got {rate}");
    }

    #[test]
    fn the_pace_returns_to_native_when_the_link_recovers() {
        let pace = LinkPace::default();
        for _ in 0..200 {
            pace.observe(Duration::from_millis(40));
        }
        for _ in 0..400 {
            pace.observe(Duration::ZERO);
        }
        assert!((pace.frame_rate() - NATIVE_FPS).abs() < 1.0);
    }

    #[test]
    fn a_queue_hands_back_only_the_kind_asked_for() {
        let queue = Queue::default();
        queue.push(Frame { kind: Kind::Packet, aid: 0, timestamp: 1, payload: b"beacon".to_vec() });
        queue.push(Frame { kind: Kind::Cmd, aid: 0, timestamp: 2, payload: b"round".to_vec() });
        let cmd = queue.pop(None, |frame| frame.kind == Kind::Cmd).expect("the CMD");
        assert_eq!(cmd.payload, b"round");
        // The beacon is untouched behind it.
        let beacon = queue.pop(None, |frame| frame.kind == Kind::Packet).expect("the beacon");
        assert_eq!(beacon.payload, b"beacon");
        assert!(queue.pop(None, |_| true).is_none());
    }

    #[test]
    fn a_non_blocking_pop_does_not_wait() {
        let queue = Queue::default();
        let started = std::time::Instant::now();
        assert!(queue.pop(None, |_| true).is_none());
        assert!(started.elapsed() < Duration::from_millis(50));
    }

    // -- the latency harness -------------------------------------------------
    //
    // Everything above tests a piece in isolation. What actually has to be
    // shown is that a *link* which used to fail now works, so the two ends are
    // run for real over a relay that adds the delay, jitter and loss a VPN
    // adds. The relay is the only way to get that evidence without two machines
    // and a tunnel between them.

    /// A UDP relay that forwards between a host and a guest, delaying every
    /// datagram and dropping some.
    ///
    /// Both directions share one socket: the host's address is known up front,
    /// and the guest's is learned from the first datagram that is not from the
    /// host — which is exactly how a NAT on the path behaves, so the transport
    /// is being exercised under the same address rewriting a real VPN imposes.
    struct Relay {
        addr: std::net::SocketAddr,
        shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl Relay {
        /// Forward to `host`, adding `delay` ± `jitter` each way and dropping
        /// `loss_percent` of datagrams.
        fn start(
            host: std::net::SocketAddr,
            delay: Duration,
            jitter: Duration,
            loss_percent: u32,
        ) -> Self {
            use std::sync::atomic::{AtomicBool, Ordering};
            let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("a relay port");
            let addr = socket.local_addr().expect("the relay address");
            socket.set_read_timeout(Some(Duration::from_millis(20))).expect("a relay read timeout");
            let shutdown = std::sync::Arc::new(AtomicBool::new(false));

            // Datagrams waiting out their delay, earliest first. Pushed by the
            // reader, drained by the writer.
            type Queued = (std::time::Instant, std::net::SocketAddr, Vec<u8>);
            let queue: std::sync::Arc<std::sync::Mutex<Vec<Queued>>> = Default::default();

            let socket = std::sync::Arc::new(socket);
            for reading in [true, false] {
                let (socket, queue, shutdown) = (
                    std::sync::Arc::clone(&socket),
                    std::sync::Arc::clone(&queue),
                    std::sync::Arc::clone(&shutdown),
                );
                std::thread::spawn(move || {
                    let mut guest: Option<std::net::SocketAddr> = None;
                    let mut buffer = vec![0u8; 4096];
                    // A cheap deterministic-enough source of variation; the
                    // point is that the delay is not constant, not that it is
                    // statistically anything in particular.
                    let mut noise: u32 = 0x9E37_79B9;
                    while !shutdown.load(Ordering::Relaxed) {
                        if reading {
                            let Ok((len, from)) = socket.recv_from(&mut buffer) else { continue };
                            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            if loss_percent > 0 && (noise >> 16) % 100 < loss_percent {
                                continue;
                            }
                            let to = if from == host {
                                match guest {
                                    Some(guest) => guest,
                                    None => continue,
                                }
                            } else {
                                guest = Some(from);
                                host
                            };
                            let spread = jitter.as_micros().max(1) as u32;
                            let extra = Duration::from_micros(u64::from((noise >> 8) % spread));
                            let at = std::time::Instant::now() + delay + extra;
                            queue.lock().unwrap().push((at, to, buffer[..len].to_vec()));
                        } else {
                            let now = std::time::Instant::now();
                            let due: Vec<Queued> = {
                                let mut queue = queue.lock().unwrap();
                                let (due, rest) =
                                    queue.drain(..).partition::<Vec<_>, _>(|(at, ..)| *at <= now);
                                *queue = rest;
                                due
                            };
                            for (_, to, bytes) in due {
                                let _ = socket.send_to(&bytes, to);
                            }
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                });
            }
            Self { addr, shutdown }
        }
    }

    impl Drop for Relay {
        fn drop(&mut self) {
            self.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// What one harness run measured.
    #[derive(Debug)]
    struct RunResult {
        rounds: u32,
        /// Rounds where every addressed client's bit came back set. This is
        /// what `mp_recv_replies` reports to the core, and on its own it is
        /// **not** enough: a reply from the previous round satisfies the mask
        /// just as well as the right one.
        answered: u32,
        /// Rounds where the reply actually carried *this* round's data.
        ///
        /// The number that matters. The guest echoes the round number it was
        /// asked about, so a reply that answers an earlier round is visible
        /// here as a mismatch — which in a game is a desynchronised link and
        /// then a communication error, even though the core was told the round
        /// succeeded.
        correct: u32,
        /// The frame rate the rounds actually came out at, which is the number
        /// the user sees as "FPS".
        effective_fps: f64,
        stats: super::LinkStats,
    }

    /// Run `rounds` CMD/reply exchanges across a relay with the given delay,
    /// pacing each round as an emulated frame would.
    fn run_rounds(
        tuning: Tuning,
        rounds: u32,
        delay: Duration,
        jitter: Duration,
        loss_percent: u32,
    ) -> RunResult {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        let host_socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("a host port");
        let host_addr = host_socket.local_addr().expect("the host address");
        drop(host_socket);
        let relay = Relay::start(host_addr, delay, jitter, loss_percent);

        let accepting = std::thread::spawn(move || {
            super::LanHost::accept(host_addr, tuning).expect("the host accepts")
        });
        // The guest reaches the host only through the relay, so the address it
        // is given is the relay's.
        let guest = super::LanGuest::connect("127.0.0.1:0".parse().unwrap(), relay.addr, tuning)
            .expect("the guest connects");
        let host = accepting.join().expect("the accept thread");

        // Let a few probes complete before any round is timed: a budget derived
        // from no measurement at all is just the floor, and would make this
        // test measure the wrong thing.
        std::thread::sleep(Duration::from_millis(1200));

        let stop = Arc::new(AtomicBool::new(false));
        let answering = {
            let (guest, stop) = (guest, Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut buffer = vec![0u8; 4096];
                while !stop.load(Ordering::Relaxed) {
                    let mut timestamp = 0;
                    if let Some(len) = guest.peer.recv_host_packet(&mut buffer, &mut timestamp) {
                        // Real client hardware answers within microseconds of
                        // the CMD it just received, and stamps the reply with
                        // its own clock — which by then reads a shade past the
                        // host's. The payload is echoed so the host can tell
                        // *which* round it is being answered about.
                        let echo = buffer[..len as usize].to_vec();
                        guest.peer.send(super::Kind::Reply, &echo, timestamp + 8, 1);
                    }
                }
                guest
            })
        };

        // AID 1, the only client here.
        let aidmask = 0b10u16;
        let frame_time = Duration::from_secs_f64(1.0 / super::NATIVE_FPS);
        let mut answered = 0;
        let mut correct = 0;
        let started = std::time::Instant::now();
        let mut slot = std::time::Instant::now();
        for round in 0..rounds {
            // The emulated wifi clock, which advances one frame per round.
            let timestamp = u64::from(round) * 16_716;
            // The CMD names the round, so the echo in the reply says which
            // round the host was actually answered about.
            host.peer.send(super::Kind::Cmd, &round.to_le_bytes(), timestamp, 0);
            let mut replies = vec![0u8; 16 * 1024];
            if host.peer.recv_replies(&mut replies, timestamp, aidmask) & aidmask == aidmask {
                answered += 1;
                // AID 1's slot is the first, at offset 0.
                if replies[..4] == round.to_le_bytes() {
                    correct += 1;
                }
            }
            // The rest of the emulated frame, if the round left any of it.
            slot += frame_time;
            let now = std::time::Instant::now();
            if slot > now {
                std::thread::sleep(slot - now);
            } else {
                slot = now;
            }
        }
        let elapsed = started.elapsed();
        stop.store(true, Ordering::Relaxed);
        let guest = answering.join().expect("the answering thread");
        drop(guest);

        RunResult {
            rounds,
            answered,
            correct,
            effective_fps: f64::from(rounds) / elapsed.as_secs_f64(),
            stats: host.stats(),
        }
    }

    /// The headline claim, measured rather than asserted: over a link whose
    /// round trip exceeds `melonds::lan`'s fixed 25 ms budget, the fixed budget
    /// collects almost nothing — which is the communication error — and the
    /// measured budget collects almost everything.
    ///
    /// The two runs differ **only** in [`Tuning`]; the transport, the relay and
    /// the round loop are the same code. `melonds::lan`'s behaviour is
    /// reproduced by pinning the budget to its 25 ms and turning off redundancy
    /// and batching, which is what that crate does by construction.
    #[test]
    fn a_measured_budget_survives_a_link_a_fixed_25ms_budget_cannot() {
        // 40 ms each way: an ordinary consumer VPN between two countries, and
        // comfortably past the 25 ms ceiling.
        let (delay, jitter) = (Duration::from_millis(40), Duration::from_millis(8));

        let fixed = Tuning {
            min_budget_ms: 25,
            max_budget_ms: 25,
            jitter_factor: 0,
            reply_copies: 1,
            batch_window_ms: 0,
            pace_to_link: false,
        };
        let before = run_rounds(fixed, 30, delay, jitter, 0);
        let after = run_rounds(Tuning::default(), 30, delay, jitter, 0);

        // Printed so `cargo test -- --nocapture` is a measurement report rather
        // than a pass/fail, which is what makes it usable as evidence.
        println!("fixed 25ms budget: {before:#?}");
        println!("measured budget:   {after:#?}");

        // Note what `before.answered` does *not* say. A fixed budget still
        // reports rounds as answered, because a reply that arrives a round late
        // sets the same bit — which is exactly why this failure shows up inside
        // a game as a desync rather than as an obviously dead link. What has to
        // be compared is `correct`: whether the host got *this* round's data.
        assert!(
            before.correct * 4 < before.rounds,
            "a fixed 25 ms budget should get almost no round's own data back over \
             an 80 ms link, but got {}/{} correct ({} reported answered)",
            before.correct,
            before.rounds,
            before.answered
        );
        assert!(
            after.correct * 10 >= after.rounds * 9,
            "a measured budget should get at least 90% of rounds' own data back over \
             an 80 ms link, but got {}/{}",
            after.correct,
            after.rounds
        );
        assert!(
            after.stats.rtt_ms > 60.0,
            "the probe should have measured the ~80 ms round trip, got {} ms",
            after.stats.rtt_ms
        );
    }

    /// Redundant replies are the answer to packet loss, which a VPN has and a
    /// LAN mostly does not. With 15% of datagrams dropped, one copy per reply
    /// loses roughly one round in seven; two copies lose roughly one in fifty.
    #[test]
    fn redundant_replies_survive_a_lossy_link() {
        let (delay, jitter) = (Duration::from_millis(20), Duration::from_millis(5));
        let single = Tuning { reply_copies: 1, ..Tuning::default() };
        let doubled = Tuning { reply_copies: 2, ..Tuning::default() };

        let one = run_rounds(single, 40, delay, jitter, 15);
        let two = run_rounds(doubled, 40, delay, jitter, 15);
        println!("one copy:  {one:#?}");
        println!("two copies: {two:#?}");

        assert!(
            two.correct >= one.correct,
            "redundancy must not make a lossy link worse: {} vs {}",
            two.correct,
            one.correct
        );
        assert!(
            two.correct * 10 >= two.rounds * 8,
            "two copies should still get 80% of rounds' own data through 15% loss, got {}/{}",
            two.correct,
            two.rounds
        );
    }

    /// The link-paced clock is what turns the remaining latency into "runs a
    /// little slow" instead of "drops rounds". Over an 80 ms link the console
    /// cannot manage 59.83 frames a second, and the pace has to say so.
    #[test]
    fn the_pace_reports_what_a_slow_link_affords() {
        let result = run_rounds(
            Tuning::default(),
            30,
            Duration::from_millis(40),
            Duration::from_millis(8),
            0,
        );
        println!("paced run: {result:#?}");
        assert!(
            result.stats.sustainable_fps < 30.0,
            "an 80 ms link cannot sustain 59.83 fps, but the pace claims {}",
            result.stats.sustainable_fps
        );
        // And the rounds really did come out at about that rate, which is what
        // makes the reported figure worth pacing to.
        assert!(
            (result.effective_fps - f64::from(result.stats.sustainable_fps)).abs() < 8.0,
            "the reported pace {} should match the observed {}",
            result.stats.sustainable_fps,
            result.effective_fps
        );
    }

    #[test]
    fn tuning_clamps_a_hand_edited_file() {
        let mut tuning = Tuning {
            min_budget_ms: 900,
            max_budget_ms: 2,
            jitter_factor: 200,
            reply_copies: 0,
            batch_window_ms: 5000,
            pace_to_link: true,
        };
        tuning.normalize();
        assert_eq!(tuning.min_budget_ms, 200);
        assert!(tuning.max_budget_ms >= tuning.min_budget_ms);
        assert_eq!(tuning.jitter_factor, 16);
        assert_eq!(tuning.reply_copies, 1);
        assert_eq!(tuning.batch_window_ms, 50);
    }
}
