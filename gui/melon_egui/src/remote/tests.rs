//! What the pieces do once they are assembled.
//!
//! Each module tests itself; these are the claims that only mean anything for
//! the whole: that a frame survives the codec, that loss heals, that a real
//! session works over a link like the one that broke LAN mode, and what all of
//! it costs.

use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use super::{
    Decoder, Encoder, RemoteClient, RemoteHost, SCREEN_HEIGHT, SCREEN_WIDTH, Tuning,
    colour::{from_565, to_565},
    tile::TILE_COUNT,
};

/// One screen's worth of framebuffer pixels, in the core's `0x00RRGGBB`.
fn screen(fill: impl Fn(usize, usize) -> u32) -> Vec<u32> {
    (0..SCREEN_WIDTH * SCREEN_HEIGHT).map(|at| fill(at % SCREEN_WIDTH, at / SCREEN_WIDTH)).collect()
}

/// The picture a decoder would show. Reference frames go through 565 first,
/// since that is the codec's declared precision and not a defect.
fn quantised(pixels: &[u32]) -> Vec<u32> {
    pixels.iter().map(|p| from_565(to_565(*p))).collect()
}

/// Roughly what a DS game looks like: a flat background, a large scrolling
/// area, and a small sprite moving quickly.
fn game_frame(frame: usize) -> (Vec<u32>, Vec<u32>) {
    let scroll = frame * 2;
    let top = screen(|x, y| {
        let stripe = ((x + scroll) / 32).is_multiple_of(2);
        let bg = if stripe { 0x0020_4060 } else { 0x0018_3850 };
        let sprite = frame * 3 % (SCREEN_WIDTH - 32);
        if x >= sprite && x < sprite + 32 && (80..112).contains(&y) { 0x00E0_C040 } else { bg }
    });
    // A menu: mostly still, which is what most of a DS bottom screen is.
    let bottom = screen(|x, y| if y % 48 < 4 || x < 8 { 0x0080_8080 } else { 0x0010_1018 });
    (top, bottom)
}

#[test]
fn a_first_frame_arrives_complete() {
    let top = screen(|x, y| ((x as u32) << 16) | ((y as u32) << 8) | 0x40);
    let bottom = screen(|x, y| ((y as u32) << 16) | 0x2000 | (x as u32));

    let mut encoder = Encoder::new(8);
    let mut datagrams = Vec::new();
    let cost = encoder.encode(&top, &bottom, &mut datagrams);
    assert_eq!(cost.tiles, TILE_COUNT, "the first frame must send every tile");

    let mut decoder = Decoder::new();
    for datagram in &datagrams {
        assert!(decoder.apply(datagram));
    }
    let [got_top, got_bottom] = decoder.take_screens().expect("a painted frame");
    assert_eq!(got_top, quantised(&top));
    assert_eq!(got_bottom, quantised(&bottom));
}

/// Every datagram must stand alone — that is what removes the need for
/// acknowledgements, keyframes and reassembly.
#[test]
fn each_datagram_is_applicable_on_its_own() {
    let top = screen(|x, y| ((x as u32) << 16) | ((y as u32) << 8));
    let bottom = screen(|x, y| ((x as u32 ^ y as u32) << 8) | 0x11);

    let mut encoder = Encoder::new(8);
    let mut datagrams = Vec::new();
    encoder.encode(&top, &bottom, &mut datagrams);
    assert!(datagrams.len() > 4, "the test needs a frame that spans several datagrams");

    // Applied in reverse, with one thrown away: every survivor still paints.
    let mut decoder = Decoder::new();
    for datagram in datagrams.iter().rev().skip(1) {
        assert!(decoder.apply(datagram));
    }
    assert!(decoder.take_screens().is_some());
}

/// The loss-recovery claim, measured: whatever is dropped, the rolling refresh
/// repaints it within one period and the picture converges exactly.
#[test]
fn a_lossy_link_converges_within_one_refresh_period() {
    const PERIOD: u8 = 8;
    let top = screen(|x, y| ((x as u32) << 16) | ((y as u32) << 8) | 0x33);
    let bottom = screen(|x, y| ((y as u32) << 16) | ((x as u32) << 8) | 0x77);

    let mut encoder = Encoder::new(PERIOD);
    let mut decoder = Decoder::new();
    let mut datagrams = Vec::new();
    let mut noise: u32 = 0x1234_5678;
    for frame in 0..(u32::from(PERIOD) * 3) {
        encoder.encode(&top, &bottom, &mut datagrams);
        for datagram in &datagrams {
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            // A third of every frame is thrown away — far worse than any usable
            // link — until the last period, which is delivered intact. That is
            // what "within one refresh period" means.
            if frame < u32::from(PERIOD) * 2 && (noise >> 16).is_multiple_of(3) {
                continue;
            }
            decoder.apply(datagram);
        }
    }
    let [got_top, got_bottom] = decoder.take_screens().expect("a painted frame");
    assert_eq!(got_top, quantised(&top), "the top screen did not converge");
    assert_eq!(got_bottom, quantised(&bottom), "the bottom screen did not converge");
}

/// Numbers rather than assertions: what the codec costs on DS-like content, and
/// what the audio rate and the frame skipping take off the total. Printed by
/// `cargo test -- --nocapture`, so the bandwidth claims are measured.
#[test]
fn the_stream_reports_what_it_costs() {
    const FRAMES: usize = 120;
    let mut encoder = Encoder::new(8);
    let mut datagrams = Vec::new();
    let (top, bottom) = game_frame(0);
    let first = encoder.encode(&top, &bottom, &mut datagrams);

    let mut bytes = 0usize;
    for frame in 1..FRAMES {
        let (top, bottom) = game_frame(frame);
        bytes += encoder.encode(&top, &bottom, &mut datagrams).bytes;
    }
    let per_frame = bytes as f64 / (FRAMES - 1) as f64;
    let mbit = |fps: f64| per_frame * 8.0 * fps / 1_000_000.0;
    let audio_mbit = |rate: u32| f64::from(rate) * 2.0 * 16.0 / 1_000_000.0;

    println!("first frame        {:>8} B / {} tiles", first.bytes, first.tiles);
    println!("steady frame       {per_frame:>8.0} B");
    println!("video @ 59.83 fps  {:>8.2} Mbit/s   (every frame)", mbit(super::NATIVE_FPS));
    println!("video @ 30 fps     {:>8.2} Mbit/s   (the default)", mbit(30.0));
    println!("video @ 10 fps     {:>8.2} Mbit/s   (the adaptive floor)", mbit(10.0));
    println!("audio @ 48 kHz     {:>8.2} Mbit/s   (untouched)", audio_mbit(48_000));
    println!("audio @ 24 kHz     {:>8.2} Mbit/s   (the default)", audio_mbit(24_000));
    println!(
        "total, defaults    {:>8.2} Mbit/s   (was {:.2} before either)",
        mbit(30.0) + audio_mbit(24_000),
        mbit(super::NATIVE_FPS) + audio_mbit(48_000),
    );

    // Halving both must be a real halving, not a rounding.
    let before = mbit(super::NATIVE_FPS) + audio_mbit(48_000);
    let after = mbit(30.0) + audio_mbit(24_000);
    assert!(after * 1.8 < before, "the defaults saved only {:.0}%", (1.0 - after / before) * 100.0);
}

// -- a real session, over a link like the one that broke LAN mode ------------

/// A UDP relay that forwards between a host and a client, delaying every
/// datagram and dropping some.
///
/// Both directions share one socket: the host's address is known up front and
/// the client's is learned from the first datagram that is not from the host —
/// which is how a NAT on the path behaves, so the transport is exercised under
/// the same address rewriting a real VPN imposes.
struct Relay {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
}

/// One datagram waiting out its delay.
type Queued = (Instant, SocketAddr, Vec<u8>);

impl Relay {
    fn start(host: SocketAddr, delay: Duration, jitter: Duration, loss_percent: u32) -> Self {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").expect("a relay port"));
        let addr = socket.local_addr().expect("the relay address");
        socket.set_read_timeout(Some(Duration::from_millis(20))).expect("a relay timeout");
        let stop = Arc::new(AtomicBool::new(false));
        let queue: Arc<Mutex<Vec<Queued>>> = Arc::default();

        for reading in [true, false] {
            let (socket, queue, stop) =
                (Arc::clone(&socket), Arc::clone(&queue), Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut client: Option<SocketAddr> = None;
                let mut buffer = vec![0u8; 4096];
                let mut noise: u32 = 0x9E37_79B9;
                while !stop.load(Ordering::Relaxed) {
                    if !reading {
                        let now = Instant::now();
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
                        continue;
                    }
                    let Ok((len, from)) = socket.recv_from(&mut buffer) else { continue };
                    noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    if (noise >> 16) % 100 < loss_percent {
                        continue;
                    }
                    let to = if from == host {
                        match client {
                            Some(client) => client,
                            None => continue,
                        }
                    } else {
                        client = Some(from);
                        host
                    };
                    let spread = jitter.as_micros().max(1) as u32;
                    let extra = Duration::from_micros(u64::from((noise >> 8) % spread));
                    queue.lock().unwrap().push((
                        Instant::now() + delay + extra,
                        to,
                        buffer[..len].to_vec(),
                    ));
                }
            });
        }
        Self { addr, stop }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A `RemoteHost` and a `RemoteClient` really talking, over a relay that adds a
/// VPN's delay, jitter and loss.
///
/// The claim that matters: a session survives a link LAN mode could not,
/// because nothing here waits for a round trip. What is checked is what a
/// player would check — the picture arrives and is correct, the controls get
/// back, the sound plays, and the link is measured.
#[test]
fn a_session_survives_a_lossy_delayed_link() {
    // 20 ms each way with 8% loss: worse than the 16.9 ms VPN that LAN mode was
    // already failing on.
    let probe = UdpSocket::bind("127.0.0.1:0").expect("a host port");
    let host_addr = probe.local_addr().expect("the host address");
    drop(probe);
    let relay = Relay::start(host_addr, Duration::from_millis(20), Duration::from_millis(4), 8);

    let tuning = Tuning {
        refresh_period: 8,
        // Every frame, so the test measures the transport rather than the
        // pacer — which has tests of its own.
        max_video_fps: 60,
        min_video_fps: 60,
        ..Tuning::default()
    };
    let accepting =
        std::thread::spawn(move || RemoteHost::accept(host_addr, tuning).expect("the host"));
    let client = RemoteClient::connect("127.0.0.1:0".parse().unwrap(), relay.addr, tuning)
        .expect("the client connects");
    let host = accepting.join().expect("the accept thread");

    let top = screen(|x, y| ((x as u32) << 16) | ((y as u32) << 8) | 0x55);
    let bottom = screen(|x, y| ((y as u32) << 16) | ((x as u32) << 8) | 0x99);
    let frame_time = Duration::from_secs_f64(1.0 / super::NATIVE_FPS);
    let mut slot = Instant::now();
    for frame in 0..90 {
        host.send_frame(&top, &bottom);
        if frame % 4 == 0 {
            // What four frames of 48 kHz stereo look like.
            host.send_audio(&vec![0i16; 800 * 2 * 4]);
        }
        // The client's side of a repaint: controls out first, picture in after.
        client.send_input(0x0F, Some((100, 50)));
        let _ = client.take_screens();
        slot += frame_time;
        let now = Instant::now();
        if slot > now {
            std::thread::sleep(slot - now);
        } else {
            slot = now;
        }
    }
    // Long enough for the last frames and a probe to complete the path.
    std::thread::sleep(Duration::from_millis(700));
    let _ = client.take_screens();
    host.send_frame(&top, &bottom);
    std::thread::sleep(Duration::from_millis(200));

    let host_stats = host.stats();
    let client_stats = client.stats();
    println!("host:   {host_stats:#?}");
    println!("client: {client_stats:#?}");

    let [got_top, got_bottom] = client.take_screens().expect("a picture");
    assert_eq!(got_top, quantised(&top), "the top screen did not converge");
    assert_eq!(got_bottom, quantised(&bottom), "the bottom screen did not converge");

    let input = host.input();
    assert_eq!(input.keys, 0x0F);
    assert_eq!(input.touch, Some((100, 50)));
    assert!(host_stats.inputs > 50, "only {} input samples arrived", host_stats.inputs);

    assert!(client_stats.audio_pairs > 0, "no audio reached the client");
    assert_eq!(
        client_stats.audio_rate,
        Tuning::default().audio_rate,
        "the client must learn the transport rate from the datagrams"
    );

    assert!(
        host_stats.rtt_ms > 25.0 && host_stats.rtt_ms < 120.0,
        "the probe measured {} ms across a ~40 ms relay",
        host_stats.rtt_ms
    );
}

/// The pacer, over the same relay: a capped frame rate must show up as frames
/// genuinely not sent, and the picture must still be right.
#[test]
fn a_capped_frame_rate_sends_fewer_frames_and_still_converges() {
    let probe = UdpSocket::bind("127.0.0.1:0").expect("a host port");
    let host_addr = probe.local_addr().expect("the host address");
    drop(probe);
    let relay = Relay::start(host_addr, Duration::from_millis(5), Duration::from_millis(2), 0);

    let tuning = Tuning { max_video_fps: 20, min_video_fps: 20, ..Tuning::default() };
    let accepting =
        std::thread::spawn(move || RemoteHost::accept(host_addr, tuning).expect("the host"));
    let client = RemoteClient::connect("127.0.0.1:0".parse().unwrap(), relay.addr, tuning)
        .expect("the client connects");
    let host = accepting.join().expect("the accept thread");

    let (top, bottom) = game_frame(7);
    for _ in 0..60 {
        host.send_frame(&top, &bottom);
    }
    std::thread::sleep(Duration::from_millis(400));

    let stats = host.stats();
    println!("capped host: {stats:#?}");
    // 20 fps out of 59.83 is every third emulated frame: 20 sent, 40 skipped.
    assert!(stats.frames <= 22, "{} frames went out where about 20 were due", stats.frames);
    assert!(stats.frames_skipped >= 38, "only {} frames were skipped", stats.frames_skipped);
    assert!(
        (stats.video_fps - 19.9).abs() < 2.0,
        "the reported rate was {} where 20 was asked for",
        stats.video_fps
    );

    let [got_top, got_bottom] = client.take_screens().expect("a picture");
    assert_eq!(got_top, quantised(&top), "skipping must not corrupt the picture");
    assert_eq!(got_bottom, quantised(&bottom));
}
