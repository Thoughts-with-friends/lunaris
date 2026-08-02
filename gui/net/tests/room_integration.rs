//! Integration test: hosts a room and joins it over real localhost
//! sockets (not mocked), verifying both the TCP control-plane player list
//! and the UDP MP-relay path work end-to-end. See
//! `docs/design/design_lan.md` §14 (the equivalent of the core crate's
//! `mp_loopback` harness, but for the actual network transport rather
//! than the in-process `LoopbackTransport`).

use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use lunaris_net::{Room, RoomConfig};
use nds_core::nds::MpTransport;

fn cfg(name: &str, control_port: u16, mp_port: u16) -> RoomConfig {
    RoomConfig {
        player_name: name.to_owned(),
        room_name: "Integration Test Room".to_owned(),
        rom_fingerprint: [0xAB; 16],
        mac_suffix: [1, 2, 3],
        max_players: 4,
        control_port,
        mp_port,
    }
}

fn wait_until(mut predicate: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    predicate()
}

#[test]
fn host_and_join_see_each_other_in_the_player_list() {
    // Distinct ports per test function to avoid collisions when the test
    // binary runs multiple tests in parallel.
    let host_room = Room::host(&cfg("Host", 27064, 27065)).expect("failed to host room");
    let guest_room = Room::join(&cfg("Guest", 27064, 27066), IpAddr::V4(Ipv4Addr::LOCALHOST))
        .expect("failed to join room");

    assert_eq!(guest_room.handle.self_id(), 1);
    assert!(!guest_room.handle.is_host());
    assert!(host_room.handle.is_host());

    let host_sees_two =
        wait_until(|| host_room.handle.players().len() == 2, Duration::from_secs(2));
    assert!(host_sees_two, "host never saw the guest join");

    let guest_sees_two =
        wait_until(|| guest_room.handle.players().len() == 2, Duration::from_secs(2));
    assert!(guest_sees_two, "guest never received the updated player list");

    let names: Vec<String> = host_room.handle.players().iter().map(|p| p.name.clone()).collect();
    assert!(names.contains(&"Host".to_owned()));
    assert!(names.contains(&"Guest".to_owned()));

    guest_room.handle.leave();
    let host_sees_leave =
        wait_until(|| host_room.handle.players().len() == 1, Duration::from_secs(2));
    assert!(host_sees_leave, "host never saw the guest leave");
}

#[test]
fn mp_frames_relay_over_real_udp_sockets() {
    let mut host_room = Room::host(&cfg("Host", 27164, 27165)).expect("failed to host room");
    let mut guest_room = Room::join(&cfg("Guest", 27164, 27166), IpAddr::V4(Ipv4Addr::LOCALHOST))
        .expect("failed to join room");

    // Give the host a moment to learn the guest's UDP address from Hello
    // before the guest's first send.
    let host_knows_guest =
        wait_until(|| host_room.handle.players().len() == 2, Duration::from_secs(2));
    assert!(host_knows_guest, "host never registered the guest's UDP endpoint");

    host_room.transport.send_packet(&[1, 2, 3, 4], 1_000);

    let mut buf = [0u8; 64];
    let mut received = false;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let nds_core::nds::MpRecv::Frame { len, .. } = guest_room.transport.recv_packet(&mut buf)
        {
            assert_eq!(&buf[..len], &[1, 2, 3, 4]);
            received = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(received, "guest never received the host's UDP packet");

    guest_room.transport.send_reply(&[9, 9], 1_000, 1);
    let mut reply_buf = [0u8; 64];
    let answered = host_room.transport.recv_replies(&mut reply_buf, 1_000, 1 << 1);
    assert_eq!(answered, 1 << 1, "host never collected the guest's reply");
}
