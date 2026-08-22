//! What the transport does, including over a relay that behaves like a VPN.

use super::*;

#[cfg(test)]
// `clippy::module_inception` fires on `lan::tests::tests`; the outer module is
// the file and the inner one is what `#[cfg(test)]` needs, so the repetition is
// the language's rather than a naming choice.
#[expect(clippy::module_inception, reason = "the file is the test module")]
mod tests {
    use std::time::Duration;

    use super::{
        Frame, HEADER_LEN, Kind, LinkPace, MAX_DATAGRAM, Measurements, Queue, Tuning, decode,
        encode_into, measure::NATIVE_FPS, stamp_sequence,
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
}
