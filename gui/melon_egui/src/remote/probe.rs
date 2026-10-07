//! Asking the other machine whether it is ready, before any session exists.
//!
//! A session's own pings ([`super::session`]) say whether a *connected* peer
//! is still there. This answers the question before that: is anyone hosting
//! at the address in the Guest IP box, and are they free?
//!
//! ```text
//!  this machine (not in a session)          the other machine
//!  Prober ── Probe every PING_INTERVAL ───→ hosting, no client yet → ProbeReply(Waiting)
//!         ←──────────── ProbeReply ──────── hosting someone else   → ProbeReply(Busy)
//!                                           not hosting            → (nothing)
//! ```
//!
//! Silence is the answer "not ready": an idle machine has no socket bound to
//! the port, so there is nothing to reply. The prober runs on a thread of its
//! own so that an unreachable address never costs the UI a frame.

use std::{
    io,
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use super::{
    session::{PING_INTERVAL, would_block},
    stats::READY_WITHIN,
    wire::{self, PeerState},
};

/// The newest answer, and whom it came from.
#[derive(Clone, Copy, Debug)]
struct Answer {
    from: SocketAddr,
    at: Instant,
    state: PeerState,
    rtt: Duration,
}

#[derive(Default)]
struct Shared {
    /// Where to ask; `None` while the address box does not parse.
    target: Mutex<Option<SocketAddr>>,
    /// Off while this machine is itself hosting or in a session.
    active: AtomicBool,
    shutdown: AtomicBool,
    last: Mutex<Option<Answer>>,
}

/// What the other machine said, as far as the UI needs to know.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Readiness {
    /// Hosting and waiting: Join will connect. Carries the probe's round trip.
    Ready { rtt_ms: f32 },
    /// Hosting, but already streaming to someone else.
    Busy,
    /// No answer within [`READY_WITHIN`].
    Silent,
}

/// The background prober. Dropping it stops the thread.
pub struct Prober {
    shared: Arc<Shared>,
}

impl Prober {
    /// Start the probing thread, initially inactive and with no target.
    ///
    /// # Errors
    /// If the thread cannot be started.
    pub fn start() -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let thread_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("melon_egui-remote-probe".to_owned())
            .spawn(move || run(&thread_shared))?;
        Ok(Self { shared })
    }

    /// Point the prober at `target`. A change forgets the old answer, so a
    /// reply from the previous address cannot be shown against the new one.
    pub fn set_target(&self, target: Option<SocketAddr>) {
        let mut current = self.shared.target.lock().unwrap_or_else(|e| e.into_inner());
        if *current != target {
            *current = target;
            *self.shared.last.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    /// Probe (`true`) or stay quiet (`false`).
    pub fn set_active(&self, active: bool) {
        self.shared.active.store(active, Ordering::Relaxed);
    }

    /// What the current target last said, judged now.
    #[must_use]
    pub fn readiness(&self) -> Readiness {
        let target = *self.shared.target.lock().unwrap_or_else(|e| e.into_inner());
        let last = *self.shared.last.lock().unwrap_or_else(|e| e.into_inner());
        match last {
            Some(answer) if Some(answer.from) == target && answer.at.elapsed() <= READY_WITHIN => {
                match answer.state {
                    PeerState::Waiting => {
                        Readiness::Ready { rtt_ms: answer.rtt.as_secs_f32() * 1000.0 }
                    }
                    PeerState::Busy => Readiness::Busy,
                }
            }
            _ => Readiness::Silent,
        }
    }
}

impl Drop for Prober {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
    }
}

/// The thread: send a probe every [`PING_INTERVAL`], collect replies between.
fn run(shared: &Shared) {
    // One socket per address family, made when first needed: a socket bound to
    // 0.0.0.0 cannot reach an IPv6 host.
    let mut socket: Option<(bool, UdpSocket)> = None;
    let mut buffer = [0u8; 64];
    let mut next_probe = Instant::now();
    while !shared.shutdown.load(Ordering::Relaxed) {
        let target = *shared.target.lock().unwrap_or_else(|e| e.into_inner());
        let Some(target) = target.filter(|_| shared.active.load(Ordering::Relaxed)) else {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };

        let v6 = target.is_ipv6();
        if socket.as_ref().is_none_or(|(is_v6, _)| *is_v6 != v6) {
            socket = bind(v6).ok().map(|s| (v6, s));
        }
        let Some((_, sock)) = &socket else {
            std::thread::sleep(PING_INTERVAL);
            continue;
        };

        if Instant::now() >= next_probe {
            // Unreachable is an answer ("silent"), not a failure.
            let _ = sock.send_to(&wire::encode_probe(wire::wall_clock_micros()), target);
            next_probe = Instant::now() + PING_INTERVAL;
        }
        match sock.recv_from(&mut buffer) {
            Ok((len, from)) if from == target => {
                if let Some((sent, state)) = wire::read_probe_reply(&buffer[..len]) {
                    let rtt = wire::wall_clock_micros().saturating_sub(sent);
                    *shared.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(Answer {
                        from,
                        at: Instant::now(),
                        state,
                        rtt: Duration::from_micros(rtt),
                    });
                }
            }
            Ok(_) => {}
            Err(ref error) if would_block(error) => {
                // A reset returns at once; without this the loop would spin
                // until the next probe is due.
                if error.kind() == io::ErrorKind::ConnectionReset {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            Err(_) => std::thread::sleep(PING_INTERVAL),
        }
    }
}

fn bind(v6: bool) -> io::Result<UdpSocket> {
    let any: SocketAddr = if v6 { "[::]:0" } else { "0.0.0.0:0" }.parse().expect("a literal");
    let socket = UdpSocket::bind(any)?;
    // Short, so a target change or shutdown is noticed promptly.
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    Ok(socket)
}
