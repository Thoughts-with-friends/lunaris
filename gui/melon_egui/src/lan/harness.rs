//! A real pair of ends, over a relay that behaves like a VPN.
//!
//! Everything in [`super::tests`] checks a piece in isolation. What actually
//! has to be shown is that a *link* which used to fail now works, so the two
//! ends are run for real over a relay that adds the delay, jitter and loss a
//! VPN adds. This is the only way to get that evidence without two machines
//! and a tunnel between them.

use super::*;

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::Tuning;

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
        let frame_time = Duration::from_secs_f64(1.0 / super::measure::NATIVE_FPS);
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
