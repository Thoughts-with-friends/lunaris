//! Headless tests of the real App driven by clicks (freya-testing), and of
//! the File menu features on a real ROM.
//!
//! The ROM is `test_roms.json`'s `default` (see [`crate::test_roms`]); its
//! folder is the ROM folder the cards are listed from.
use std::{
    any::Any,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use ds_common::{Instance, Settings, frontend::lock};
use freya::{elements::label::LabelElement, prelude::Size2D};
use freya_testing::prelude::*;

use crate::{App, backend::BACKEND, frontend::Frontend};

fn rom() -> PathBuf {
    crate::test_roms::rom("default")
}

fn rom_dir() -> PathBuf {
    rom().parent().expect("a ROM folder").to_owned()
}

/// The ROM's card label (its file name).
fn rom_name() -> String {
    rom().file_name().expect("a file name").to_string_lossy().into_owned()
}

/// A frontend in a fresh instance folder whose ROM folder is `rom_dir`.
fn front(name: &str) -> Arc<Frontend> {
    let base = std::env::temp_dir().join(format!("lunaris_freya_ui_{name}_{BACKEND}"));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("temp dir");
    let instance = Instance::open_in(&base);
    let config = serde_json::json!({ "romDir": rom_dir() });
    std::fs::write(Frontend::config_path(&instance), config.to_string()).expect("config");
    Arc::new(Frontend::new(instance, Settings::default()))
}

fn runner(front: &Arc<Frontend>) -> TestingRunner {
    let app = App { front: Arc::clone(front), dialog: None };
    let (mut t, ()) = TestingRunner::new(move || app.clone(), Size2D::new(1000., 800.), |_| {}, 1.);
    t.sync_and_update();
    t
}

fn labels(t: &TestingRunner) -> Vec<String> {
    t.find_many(|_, el| (el as &dyn Any).downcast_ref::<LabelElement>().map(|l| l.text.to_string()))
}

/// Clicks the first label that is exactly `text`.
fn click(t: &mut TestingRunner, text: &str) {
    let pos = t
        .find(|node, el| {
            let label = (el as &dyn Any).downcast_ref::<LabelElement>()?;
            (label.text == text).then(|| {
                let c = node.layout().area.center();
                (f64::from(c.x), f64::from(c.y))
            })
        })
        .unwrap_or_else(|| panic!("no {text:?} in {:?}", labels(t)));
    t.click_cursor(pos);
    t.poll(Duration::from_millis(10), Duration::from_millis(150));
}

#[test]
#[ignore = "requires a ROM (test_roms.json)"]
fn rom_card_and_menus_work() {
    let front = front("cards");
    let mut t = runner(&front);
    click(&mut t, &rom_name());
    assert_eq!(lock(&front.rom).clone(), Some(rom()), "card starts");
    click(&mut t, "System");
    click(&mut t, "Pause");
    assert!(front.shared.is_paused(), "System > Pause");
    click(&mut t, "File");
    assert!(labels(&t).contains(&"Export savefile...".to_owned()), "File > Export savefile");
    front.stop();
}

/// Waits until the emulator thread has made `n` more frames.
fn run_frames(front: &Frontend, n: u64) {
    let until = front.shared.frames() + n;
    let start = Instant::now();
    while front.shared.frames() < until {
        assert!(start.elapsed() < Duration::from_secs(30), "emulator stalled");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "requires a ROM (test_roms.json)"]
fn savestates_and_export_work() {
    use crate::frontend::Command;

    let front = front("states");
    let rom = rom();
    front.start(&rom).expect("ROM loads");
    run_frames(&front, 30);

    if BACKEND.has_states() {
        let slot = |n| front.state_path(n).expect("core has states");
        front.command(Command::SaveState(slot(1))).expect("save state");
        assert!(slot(1).is_file(), "slot 1 written");
        run_frames(&front, 30);
        front.load_state(slot(1)).expect("load state");
        run_frames(&front, 10);
        assert!(front.undo_path().is_file(), "undo point kept");
        front.undo_state_load().expect("undo");
        assert!(!front.undo_path().is_file(), "undo used up");
        run_frames(&front, 10);
        assert!(front.command(Command::LoadState(slot(2))).is_err(), "empty slot");
    } else {
        assert!(front.state_path(1).is_none());
    }

    let dest = std::env::temp_dir().join(format!("lunaris_freya_export_{BACKEND}.sav"));
    let _ = std::fs::remove_file(&dest);
    match front.export_save(&dest) {
        Ok(()) => assert!(dest.is_file(), "exported"),
        Err(e) => assert!(e.contains("has not saved"), "{e}"),
    }
    front.stop();
    assert!(front.command(Command::FlushSave).is_err(), "no game, no command");
}

#[test]
#[ignore = "requires a ROM (test_roms.json)"]
fn debug_panel_shows_views() {
    let front = front("debug");
    let mut t = runner(&front);
    click(&mut t, &rom_name());
    run_frames(&front, 60);
    click(&mut t, "Debug");
    click(&mut t, "Performance stats");
    t.poll(Duration::from_millis(20), Duration::from_millis(600));
    assert!(labels(&t).iter().any(|l| l.starts_with("Average")), "{:?}", labels(&t));
    if BACKEND.has_debug() {
        click(&mut t, "VRAM");
        t.poll(Duration::from_millis(20), Duration::from_millis(600));
        assert!(labels(&t).iter().any(|l| l == "256×256"), "{:?}", labels(&t));
        assert!(front.shared.debug_image_since(0).is_some(), "the core drew the bank");
        click(&mut t, "Tiles");
        t.poll(Duration::from_millis(20), Duration::from_millis(400));
        if let Some(path) = std::env::var_os("FREYA_SNAPSHOT") {
            t.render_to_file(PathBuf::from(path));
        }
    }
    click(&mut t, "✕");
    front.stop();
}

/// Hosting a melonDS LAN session (one per process, so no guest here): the
/// running core gets a link, keeps running on it, state loads are refused
/// until the session is left.
#[cfg(feature = "lunaris")]
#[test]
#[ignore = "requires a ROM (test_roms.json)"]
fn lan_session_hosting() {
    let rom = rom();
    let host = front("lan_host");
    let mut config = host.lan_config();
    config.player_name = "Host".to_owned();
    host.set_lan_config(&config);
    host.start(&rom).expect("ROM loads");
    run_frames(&host, 20);
    host.host_room().expect("host");

    let start = Instant::now();
    loop {
        host.room_tick();
        let (room, _, error) = host.room_view();
        assert!(error.is_none(), "{error:?}");
        if room.is_some_and(|r| r.host && r.players.len() == 1 && r.players[0].4) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "{:?}", host.room_view());
        std::thread::sleep(Duration::from_millis(50));
    }
    // The core keeps running with the link installed.
    run_frames(&host, 30);
    assert!(host.refuse_in_room().is_err(), "no state loads in a session");

    host.leave_room();
    assert!(host.refuse_in_room().is_ok());
    assert!(host.room_view().0.is_none());
    run_frames(&host, 10);
    host.stop();
}

/// melonDS: one frontend hosts on 127.0.0.1, another joins; both restart
/// the ROM on the link and keep running, paced by it.
#[cfg(feature = "melon")]
#[test]
#[ignore = "requires a ROM (test_roms.json)"]
fn melon_link_over_localhost() {
    let rom = rom();
    let host = front("melon_lan_host");
    let guest = front("melon_lan_guest");
    host.start(&rom).expect("ROM loads");
    guest.start(&rom).expect("ROM loads");
    host.open_link(true, "127.0.0.1:47164").expect("hosting starts");
    std::thread::sleep(Duration::from_millis(100));
    guest.open_link(false, "127.0.0.1:47164").expect("joining starts");

    let start = Instant::now();
    while [&host, &guest].iter().any(|f| f.room_view().0.is_none()) {
        for f in [&host, &guest] {
            let (_, _, error) = f.room_view();
            assert!(error.is_none(), "{error:?}");
        }
        assert!(start.elapsed() < Duration::from_secs(15), "never linked");
        std::thread::sleep(Duration::from_millis(50));
    }
    run_frames(&host, 60);
    run_frames(&guest, 60);
    let (view, _, _) = host.room_view();
    let view = view.expect("linked");
    assert!(view.hosting);
    assert!(view.stats.datagrams_received > 0, "{:?}", view.stats);
    assert!(guest.refuse_in_room().is_err(), "no state loads on a link");

    guest.unlink().expect("restarts unlinked");
    assert!(guest.room_view().0.is_none());
    assert!(guest.refuse_in_room().is_ok());
    run_frames(&guest, 10);
    host.stop();
    guest.stop();
}

/// View > Video settings: 2x / 4x frames come out at that size on every
/// core, with and without xBRZ (melonDS 4x without xBRZ: OpenGL renderer).
#[test]
#[ignore = "requires a ROM (test_roms.json)"]
fn upscaled_frames_have_the_internal_resolution() {
    let front = front("hires");
    front.start(&rom()).expect("ROM loads");
    run_frames(&front, 60);
    let mut native = Vec::new();
    for (scale, xbrz) in [(1_u8, false), (2, false), (4, false), (2, true), (4, true)] {
        front.shared.set_render(scale, xbrz);
        run_frames(&front, 20);
        let start = Instant::now();
        let want = 256 * u32::from(scale);
        loop {
            let (_, width, rgba) = front.shared.frame_since(0).expect("frames");
            if width == want {
                assert_eq!(rgba.len(), (want * want * 3 / 4 * 2 * 4) as usize);
                let lit = rgba.as_chunks::<4>().0.iter().any(|p| p[..3] != [0, 0, 0]);
                assert!(lit, "{scale}x xbrz {xbrz}: a picture");
                if scale == 1 {
                    native.clone_from(&rgba);
                } else if !xbrz {
                    // Sampled back to 1x it is the same picture, the right
                    // way up (the title screen stands still): a flipped or
                    // swapped screen differs almost everywhere.
                    let n = usize::from(scale);
                    let differ = (0..256 * 384)
                        .filter(|i| {
                            let (x, y) = (i % 256, i / 256);
                            let big = &rgba[((y * n) * 256 * n + x * n) * 4..][..3];
                            let small = &native[i * 4..][..3];
                            big.iter().zip(small).any(|(a, b)| a.abs_diff(*b) > 40)
                        })
                        .count();
                    assert!(differ < 256 * 384 / 50, "{scale}x: {differ} pixels differ from 1x");
                }
                if let Some(dir) = std::env::var_os("FREYA_SNAPSHOT_DIR") {
                    let path = PathBuf::from(dir).join(format!("{BACKEND}_{scale}x_{xbrz}.rgba"));
                    std::fs::write(path, &rgba).expect("dump");
                }
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "{scale}x xbrz {xbrz}: width {width}"
            );
            run_frames(&front, 1);
        }
    }
    // Changed while paused: the paused frame comes back the new way.
    front.shared.set_paused(true);
    // The frame in progress still finishes (at 4x in a debug build, longer
    // than any fixed wait), so wait for the count to stand still.
    let mut paused_at = front.shared.frames();
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let now = front.shared.frames();
        if now == paused_at {
            break;
        }
        paused_at = now;
    }
    front.shared.set_render(2, true);
    let start = Instant::now();
    while front.shared.frame_since(0).is_none_or(|(_, width, _)| width != 512) {
        assert!(start.elapsed() < Duration::from_secs(5), "the paused frame was not redone");
        std::thread::sleep(Duration::from_millis(10));
    }
    // One republish, no emulated frames.
    assert_eq!(front.shared.frames(), paused_at + 1, "still paused");
    front.shared.set_paused(false);
    front.stop();
}

/// The Multiplayer dialog (melonDS): LAN by IP and Remote Desktop, each with
/// its knobs folded away; a press on a knob is saved for the next session.
#[cfg(feature = "melon")]
#[test]
fn multiplayer_dialog_shows_both_modes() {
    let front = front("mp_dialog");
    let app = App { front: Arc::clone(&front), dialog: Some(ds_common::menu::Window::Multiplayer) };
    let (mut t, ()) =
        TestingRunner::new(move || app.clone(), Size2D::new(1000., 1400.), |_| {}, 1.);
    t.sync_and_update();
    t.poll(Duration::from_millis(20), Duration::from_millis(300));
    let shown = labels(&t);
    for text in ["Host address", "Join host", "Partner", "VPN tuning", "Remote Desktop settings"] {
        assert!(shown.iter().any(|l| l == text), "{text:?} in {shown:?}");
    }
    assert!(shown.iter().any(|l| l.starts_with("nobody hosts at")), "{shown:?}");

    click(&mut t, "Remote Desktop settings");
    click(&mut t, "VPN tuning");
    let shown = labels(&t);
    for text in ["Refresh period", "Audio lag limit", "Jitter factor", "Pace to the link"] {
        assert!(shown.iter().any(|l| l == text), "{text:?} in {shown:?}");
    }
    if let Some(path) = std::env::var_os("FREYA_SNAPSHOT") {
        t.render_to_file(PathBuf::from(path));
    }
    // The "+" right of the refresh period.
    let plus = t
        .find(|node, el| {
            let label = (el as &dyn Any).downcast_ref::<LabelElement>()?;
            (label.text == "8 frames").then(|| {
                let a = node.layout().area;
                (f64::from(a.max_x()) + 30., f64::from(a.center().y))
            })
        })
        .expect("refresh period");
    t.click_cursor(plus);
    t.poll(Duration::from_millis(10), Duration::from_millis(150));
    assert_eq!(front.remote_config().tuning.refresh_period, 9, "{:?}", labels(&t));
    assert!(labels(&t).iter().any(|l| l == "9 frames"));
    front.pause_remote_probe();
}

/// Remote Desktop: one frontend hosts with a game running, the other joins
/// without one and shows the host's second console's picture; its keys
/// reach that console, its save / state requests are done there (on the
/// host's next instance folder). Stop ends the session on both sides.
#[test]
#[ignore = "requires a ROM (test_roms.json)"]
fn remote_desktop_over_localhost() {
    let host = front("rd_host");
    let client = front("rd_client");
    host.start(&rom()).expect("ROM loads");
    assert!(!client.remote_partner("127.0.0.1:47265").0, "nobody hosts yet");
    host.host_remote("127.0.0.1:47265").expect("hosting starts");
    // The client's probe sees the waiting host before it joins.
    let start = Instant::now();
    while !client.remote_partner("127.0.0.1:47265").0 {
        assert!(start.elapsed() < Duration::from_secs(10), "the probe never saw the host");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!host.remote_partner("127.0.0.1:47265").0, "a waiting host has no partner");
    client.join_remote("127.0.0.1:47265").expect("joining starts");

    let start = Instant::now();
    while [&host, &client].iter().any(|f| f.remote_view().0.is_none()) {
        for f in [&host, &client] {
            assert!(f.remote_view().2.is_none(), "{:?}", f.remote_view().2);
        }
        assert!(start.elapsed() < Duration::from_secs(15), "never connected");
        std::thread::sleep(Duration::from_millis(50));
    }
    // The client shows a picture it got over the stream.
    let start = Instant::now();
    loop {
        run_frames(&client, 10);
        let (_, width, rgba) = client.shared.frame_since(0).expect("frames");
        assert_eq!(width, 256);
        if rgba.as_chunks::<4>().0.iter().any(|p| p[..3] != [0, 0, 0]) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(15), "the stream never showed a picture");
    }
    client.shared.set_keys(1);
    run_frames(&client, 30);
    assert!(host.remote_partner("").0, "in a session, the session's pings answer");
    // Video settings apply to the streamed picture too (xBRZ, 2x).
    client.shared.set_render(2, true);
    let start = Instant::now();
    while client.shared.frame_since(0).is_none_or(|(_, width, _)| width != 512) {
        assert!(start.elapsed() < Duration::from_secs(10), "the stream was not upscaled");
        run_frames(&client, 1);
    }
    // The second console lives in the host's next instance folder.
    let view = host.remote_view().0.expect("hosting");
    assert_eq!(view.instance, Some(host.instance.number() + 1));
    assert!(matches!(
        client.remote_state(),
        Some(crate::remote::RemoteState::Connected { hosting: false, .. })
    ));
    // The client's save and state actions are done by the host.
    let remote = client.remote_client().expect("a client");
    let save: Vec<u8> = (0..0x2000_u32).map(|i| (i * 7) as u8).collect();
    remote.request(&::remote::Request::ImportSave(save.clone())).expect("import");
    assert_eq!(remote.request(&::remote::Request::ExportSave).expect("export"), save);
    remote.request(&::remote::Request::SaveState(3)).expect("save state");
    remote.request(&::remote::Request::LoadState(3)).expect("load state");
    assert!(remote.request(&::remote::Request::LoadState(4)).is_err(), "slot 4 is empty");
    run_frames(&client, 10);
    let host_stats = host.remote_view().0.expect("hosting").stats;
    assert!(host_stats.frames > 0, "the host sent video: {host_stats:?}");
    let client_stats = client.remote_view().0.expect("joined").stats;
    assert!(client_stats.inputs > 0, "the client sent its keys: {client_stats:?}");

    client.end_remote();
    assert!(client.remote_view().0.is_none());
    host.end_remote();
    assert!(host.remote_view().0.is_none());
    run_frames(&host, 10);
    host.stop();
}

/// Connects a host (game running) and a client on `port`.
fn remote_pair(name: &str, port: u16) -> (Arc<Frontend>, Arc<Frontend>) {
    let host = front(&format!("{name}_host"));
    let client = front(&format!("{name}_client"));
    host.start(&rom()).expect("ROM loads");
    host.host_remote(&format!("127.0.0.1:{port}")).expect("hosting starts");
    client.join_remote(&format!("127.0.0.1:{port}")).expect("joining starts");
    let start = Instant::now();
    while [&host, &client].iter().any(|f| f.remote_view().0.is_none()) {
        assert!(start.elapsed() < Duration::from_secs(15), "never connected");
        std::thread::sleep(Duration::from_millis(50));
    }
    (host, client)
}

/// Polls `front` as the status bar does until it reports the peer gone.
fn wait_peer_gone(front: &Frontend) -> String {
    let start = Instant::now();
    loop {
        if let Some(message) = front.check_remote_peer() {
            return message;
        }
        assert!(start.elapsed() < Duration::from_secs(15), "the peer never counted as gone");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The host goes away: the client is told and is back on the welcome
/// screen (no game, no session), free to start a game of its own.
#[test]
#[ignore = "requires a ROM (test_roms.json)"]
fn remote_client_returns_home_when_the_host_leaves() {
    let (host, client) = remote_pair("rd_host_leaves", 47266);
    assert!(client.check_remote_peer().is_none(), "the host still answers");
    host.stop();
    let message = wait_peer_gone(&client);
    assert!(message.contains("host"), "{message}");
    assert!(client.remote_view().0.is_none(), "the session ended");
    assert!(!client.shared.is_running(), "back to the welcome screen");
    assert!(lock(&client.rom).is_none());
    assert_eq!(client.remote_view().2.as_deref(), Some(message.as_str()), "the dialog says so");
    client.start(&rom()).expect("the client plays on its own");
    run_frames(&client, 10);
    client.stop();
}

/// The client goes away: the host is told, the session ends and the host's
/// own game keeps running.
#[test]
#[ignore = "requires a ROM (test_roms.json)"]
fn remote_host_keeps_playing_when_the_client_leaves() {
    let (host, client) = remote_pair("rd_client_leaves", 47267);
    client.stop();
    let message = wait_peer_gone(&host);
    assert!(message.contains("client"), "{message}");
    assert!(host.remote_view().0.is_none(), "the session ended");
    assert!(host.shared.is_running(), "the host's game goes on");
    run_frames(&host, 10);
    host.stop();
}
