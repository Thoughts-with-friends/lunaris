//! Two consoles in one process on melonDS's LocalMP medium, each on its own
//! thread (the MP lockstep waits need them to run concurrently).
//!
//! ```text
//! cargo run --release -p ds_net --example mp_pair -- <rom> <frames> <out_dir>
//! ```
//!
//! Env, per console `N` = 0 (first) / 1 (second):
//! - `PRESSN=key:start:every[:end],...` buttons (a b x y l r start select up
//!   down left right), each press held 6 frames
//! - `TOUCHN=x:y:start:every[:end],...` stylus, held 6 frames
//! - `SAVEN=<dir>` save folder (default `<out_dir>/cN`)
//! - `PNG_EVERY=n` (default 120) both screens -> `<out_dir>/cN/fFFFFF.png`
//! - `SPEED=1.0` real-time factor (0 = unthrottled)
//! - `BACKEND=lan` melonDS LAN backend (no session) instead of LocalMP, as
//!   the GUI runs; `CONSOLES=1` only console 0; `NOLINK=1` no link at all
//!   (for comparing a linked console against an unlinked one)
//!
//! Every 60 frames each console prints its Wi-Fi channel, AID, MP state and
//! link traffic (frames sent per kind / received); a non-zero AID on a
//! client means it associated with the host.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use ds_emu::{Emulator, MpLink};
use ds_net::{MpBackend, NetLink};
use ds_peripherals::wifi::MpRecv;

const FPS: f64 = 59.826_098;

/// Frames sent (packet, cmd, reply, ack) and received (any, host, replies).
#[derive(Debug, Default)]
struct Counters([AtomicU64; 7]);

impl Counters {
    fn bump(&self, i: usize) {
        self.0[i].fetch_add(1, Ordering::Relaxed);
    }
    fn get(&self) -> [u64; 7] {
        std::array::from_fn(|i| self.0[i].load(Ordering::Relaxed))
    }
}

/// [`NetLink`] plus traffic counters for the log.
#[derive(Debug)]
struct Counted(NetLink, Arc<Counters>);

impl MpLink for Counted {
    fn begin(&mut self) {
        self.0.begin();
    }
    fn end(&mut self) {
        self.0.end();
    }
    fn new_frame(&mut self) {
        self.0.new_frame();
    }
    fn send_packet(&mut self, frame: &[u8], ts: u64) {
        self.1.bump(0);
        self.0.send_packet(frame, ts);
    }
    fn send_cmd(&mut self, frame: &[u8], ts: u64) {
        self.1.bump(1);
        self.0.send_cmd(frame, ts);
    }
    fn send_reply(&mut self, frame: &[u8], ts: u64, aid: u16) {
        self.1.bump(2);
        self.0.send_reply(frame, ts, aid);
    }
    fn send_ack(&mut self, frame: &[u8], ts: u64) {
        self.1.bump(3);
        self.0.send_ack(frame, ts);
    }
    fn recv_packet(&mut self, buf: &mut [u8]) -> MpRecv {
        let r = self.0.recv_packet(buf);
        if matches!(r, MpRecv::Frame { .. }) {
            self.1.bump(4);
        }
        r
    }
    fn recv_host_packet(&mut self, buf: &mut [u8]) -> MpRecv {
        let r = self.0.recv_host_packet(buf);
        if matches!(r, MpRecv::Frame { .. }) {
            self.1.bump(5);
        }
        r
    }
    fn recv_replies(&mut self, buf: &mut [u8], ts: u64, mask: u16) -> u16 {
        let got = self.0.recv_replies(buf, ts, mask);
        if got != 0 {
            self.1.bump(6);
        }
        got
    }
}

type Press = (String, u32, u32, u32);
type Touch = (i32, i32, u32, u32, u32);

fn list(var: &str) -> Vec<Vec<String>> {
    std::env::var(var)
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.split(':').map(str::to_owned).collect())
        .collect()
}

fn num<T: std::str::FromStr>(s: Option<&String>, default: T) -> T {
    s.and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn key(emu: &mut Emulator, key: &str, down: bool) {
    let k = &mut emu.key_input;
    let e = &mut emu.ext_key_in;
    match key {
        "a" => k.button_a = down,
        "b" => k.button_b = down,
        "start" => k.start = down,
        "select" => k.select = down,
        "up" => k.up = down,
        "down" => k.down = down,
        "left" => k.left = down,
        "right" => k.right = down,
        "l" => k.button_l = down,
        "r" => k.button_r = down,
        "x" => e.button_x = down,
        "y" => e.button_y = down,
        _ => {}
    }
}

fn save_png(path: &std::path::Path, upper: &[u32], lower: &[u32]) {
    let rgb: Vec<u8> = upper
        .iter()
        .chain(lower)
        .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8])
        .collect();
    if let Err(e) = image::save_buffer(path, &rgb, 256, 384, image::ExtendedColorType::Rgb8) {
        eprintln!("png: {e}");
    }
}

#[allow(clippy::cognitive_complexity)]
fn console(n: usize, rom: &str, frames: u32, out: &std::path::Path) {
    let presses: Vec<Press> = list(&format!("PRESS{n}"))
        .into_iter()
        .map(|p| (p[0].clone(), num(p.get(1), 0), num(p.get(2), 1), num(p.get(3), u32::MAX)))
        .collect();
    let touches: Vec<Touch> = list(&format!("TOUCH{n}"))
        .into_iter()
        .map(|p| {
            (
                num(p.first(), 0),
                num(p.get(1), 0),
                num(p.get(2), 0),
                num(p.get(3), 1),
                num(p.get(4), u32::MAX),
            )
        })
        .collect();
    let png_every: u32 = num(std::env::var("PNG_EVERY").ok().as_ref(), 120);
    let speed: f64 = num(std::env::var("SPEED").ok().as_ref(), 1.0);
    let dir = out.join(format!("c{n}"));
    let _ = std::fs::create_dir_all(&dir);

    let mut emu = Emulator::new();
    emu.config.save_dir =
        Some(std::env::var(format!("SAVE{n}")).map_or_else(|_| dir.clone(), Into::into));
    // DS Wifi I/O Map: https://problemkaputt.de/gbatek.htm#dswifiiomap
    emu.config.mac_address = Some([0x00, 0x09, 0xBF, 0x00, 0x00, n as u8 + 1]); // Nintendo OUI 00:09:BF, last byte = console number
    let counters = Arc::new(Counters::default());
    if std::env::var("NOLINK").is_err() {
        emu.set_mp_link(Some(Box::new(Counted(
            NetLink::new(n).with_pump(n == 0).with_stall_budget(Some(Duration::from_millis(100))),
            Arc::clone(&counters),
        ))));
    }
    if let Err(e) = emu.load_rom(rom) {
        eprintln!("c{n}: {e:?}");
        return;
    }

    let (mut upper, mut lower) = (vec![0_u32; 256 * 192], vec![0_u32; 256 * 192]);
    let start = Instant::now();
    let mut last_aid = 0;
    for f in 0..frames {
        for (k, from, every, end) in &presses {
            if f >= *from && f <= *end && (f - from) % every == 0 {
                key(&mut emu, k, true);
            }
            if f >= from + 6 && f <= end.saturating_add(6) && (f - from - 6) % every == 0 {
                key(&mut emu, k, false);
            }
        }
        let pen = touches.iter().find(|(_, _, from, every, end)| {
            f >= *from && f <= end.saturating_add(6) && (f - from) % every < 6
        });
        match pen {
            Some((x, y, ..)) => emu.touchscreen_press(*x, *y),
            // DS Touch Screen Controller (TSC): https://problemkaputt.de/gbatek.htm#dstouchscreencontrollertsc
            None if !touches.is_empty() => emu.touchscreen_press(0, 0xFFF), // Y = FFFh releases the pen
            None => {}
        }

        emu.run();

        let aid = emu.wifi.aid();
        if f % 60 == 0 || aid != last_aid {
            let (host, client) = emu.wifi.mp_state();
            let [pk, cmd, rep, ack, rx, rxh, rxr] = counters.get();
            println!(
                "c{n} f={f:5} ch={:2} aid={aid} host={host} client={client}                  tx pkt/cmd/rep/ack={pk}/{cmd}/{rep}/{ack} rx any/host/replies={rx}/{rxh}/{rxr}",
                emu.wifi.channel(),
            );
            last_aid = aid;
        }
        if png_every != 0 && f % png_every == 0 {
            emu.get_upper_frame(&mut upper);
            emu.get_lower_frame(&mut lower);
            save_png(&dir.join(format!("f{f:05}.png")), &upper, &lower);
        }
        if speed > 0.0 {
            let due = start + Duration::from_secs_f64(f64::from(f + 1) / (FPS * speed));
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
    }
    emu.get_upper_frame(&mut upper);
    emu.get_lower_frame(&mut lower);
    save_png(&dir.join("last.png"), &upper, &lower);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, rom, frames, out] = args.as_slice() else {
        eprintln!("usage: mp_pair <rom> <frames> <out_dir>");
        std::process::exit(2);
    };
    let frames: u32 = frames.parse().unwrap_or(600);
    let lan = std::env::var("BACKEND").is_ok_and(|b| b == "lan");
    ds_net::init(if lan { MpBackend::Lan } else { MpBackend::Local });
    let consoles = if std::env::var("CONSOLES").is_ok_and(|c| c == "1") { 1 } else { 2 };

    let threads: Vec<_> = (0..consoles)
        .map(|n| {
            let (rom, out) = (rom.clone(), std::path::PathBuf::from(out));
            std::thread::spawn(move || console(n, &rom, frames, &out))
        })
        .collect();
    for t in threads {
        let _ = t.join();
    }
}
