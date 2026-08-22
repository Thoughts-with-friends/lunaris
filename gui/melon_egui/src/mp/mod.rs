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

mod airwaves;
mod client;
mod counters;
mod frame;

pub(crate) use airwaves::Shared;
pub use airwaves::{Airwaves, MAX_INSTANCES};
pub use client::Client;
// The receive timeout, read by the tests below that assert a blocking
// receive returned on a packet rather than on the clock.
#[cfg(test)]
pub(crate) use client::RECV_TIMEOUT;
pub use counters::Counters;
pub use frame::{Event, Kind};
pub(crate) use frame::{LOG_LIMIT, MAX_FRAME_SIZE, Packet, REPLY_SLOT};

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
