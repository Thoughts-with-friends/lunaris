//! Headless runner: `headless <rom> <frames> <out_dir>`
//! Env: SAVE_DIR, PRESS=<key>:<start>:<every> (comma list), PNG_EVERY=n, PCS=1
#[cfg(feature = "log")]
use std::sync::atomic::{AtomicBool, Ordering};

use ds_emu::Emulator;

#[cfg(feature = "log")]
static LOG_ON: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "log")]
struct L;
#[cfg(feature = "log")]
impl log::Log for L {
    fn enabled(&self, _: &log::Metadata) -> bool {
        LOG_ON.load(Ordering::Relaxed)
    }
    fn log(&self, r: &log::Record) {
        if LOG_ON.load(Ordering::Relaxed) {
            eprintln!("[{}] {}: {}", r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {}
}

fn press(emu: &mut Emulator, key: &str, down: bool) {
    match (key, down) {
        ("a", true) => emu.button_a_pressed(),
        ("a", false) => emu.button_a_released(),
        ("b", true) => emu.button_b_pressed(),
        ("b", false) => emu.button_b_released(),
        ("start", true) => emu.button_start_pressed(),
        ("start", false) => emu.button_start_released(),
        ("down", true) => emu.button_down_pressed(),
        ("down", false) => emu.button_down_released(),
        ("up", true) => emu.button_up_pressed(),
        ("up", false) => emu.button_up_released(),
        ("left", true) => emu.button_left_pressed(),
        ("left", false) => emu.button_left_released(),
        ("right", true) => emu.button_right_pressed(),
        ("right", false) => emu.button_right_released(),
        _ => {}
    }
}

#[allow(clippy::unwrap_used)]
fn save_png(path: &str, buf: &[u32]) {
    let mut rgb = Vec::with_capacity(buf.len() * 3);
    for &p in buf {
        rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
    }
    image::save_buffer(path, &rgb, 256, 192, image::ExtendedColorType::Rgb8).unwrap();
}

#[allow(clippy::unwrap_used, clippy::cognitive_complexity)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom = &args[1];
    let frames: u32 = args[2].parse().unwrap();
    let out = &args[3];
    std::fs::create_dir_all(out).unwrap();

    #[allow(unused_variables)]
    let log_range: Option<(u32, u32)> = std::env::var("LOGF").ok().map(|s| {
        let p: Vec<&str> = s.split(':').collect();
        (p[0].parse().unwrap(), p[1].parse().unwrap())
    });

    #[cfg(feature = "log")]
    log::set_logger(&L).unwrap();

    #[cfg(feature = "log")]
    log::set_max_level(log::LevelFilter::Trace);

    let mut emu = Emulator::new();
    if let Ok(hd) = std::env::var("HD") {
        emu.gpu.set_render_scale(hd.split(':').next().unwrap().parse().unwrap());
    }
    if let Ok(dir) = std::env::var("SAVE_DIR") {
        emu.config.save_dir = Some(dir.into());
    }
    if let Ok(w) = std::env::var("WATCH") {
        emu.config.watch_addr = Some(u32::from_str_radix(&w, 16).unwrap());
    }
    emu.load_rom(rom).unwrap();

    let presses: Vec<(String, u32, u32, u32)> = std::env::var("PRESS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let p: Vec<&str> = s.split(':').collect();
            let end = p.get(3).map_or(u32::MAX, |e| e.parse().unwrap());
            (p[0].to_owned(), p[1].parse().unwrap(), p[2].parse().unwrap(), end)
        })
        .collect();

    // TOUCH=x:y:start:every[:end] (comma list), pen held for 6 frames.
    let touches: Vec<(i32, i32, u32, u32, u32)> = std::env::var("TOUCH")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let p: Vec<&str> = s.split(':').collect();
            let end = p.get(4).map_or(u32::MAX, |e| e.parse().unwrap());
            (
                p[0].parse().unwrap(),
                p[1].parse().unwrap(),
                p[2].parse().unwrap(),
                p[3].parse().unwrap(),
                end,
            )
        })
        .collect();

    // POKE=hexaddr:byte:frame (comma list), ARM9 byte write before that frame.
    let pokes: Vec<(u32, u8, u32)> = std::env::var("POKE")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let p: Vec<&str> = s.split(':').collect();
            (u32::from_str_radix(p[0], 16).unwrap(), p[1].parse().unwrap(), p[2].parse().unwrap())
        })
        .collect();

    let png_every: u32 = std::env::var("PNG_EVERY").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
    let pcs_every: u32 = std::env::var("PCS").ok().and_then(|s| s.parse().ok()).unwrap_or(0);

    let mut up = vec![0_u32; 256 * 192];
    let mut lo = vec![0_u32; 256 * 192];
    let gui_mode = std::env::var("GUI").is_ok() || std::env::var("WAV").is_ok();
    let wav_out = std::env::var("WAV").ok();
    let mut wav_all: Vec<i16> = Vec::new();
    let mut audio_buf = Vec::new();
    let mut last_hash = 0_u64;
    let trace: Option<(u8, u32, u32)> = std::env::var("TRACE").ok().map(|s| {
        let p: Vec<&str> = s.split(':').collect();
        (p[0].parse().unwrap(), p[1].parse().unwrap(), p[2].parse().unwrap())
    });

    // HDBENCH=<scale>[:x]: upscale both screens every frame and time it.
    let hd_bench: Option<(usize, bool)> = std::env::var("HDBENCH").ok().map(|h| {
        let scale = h.split(':').next().unwrap().parse().unwrap();
        emu.gpu.set_render_scale(scale);
        (scale, h.ends_with(":x"))
    });
    let mut bench = (0.0_f64, 0.0_f64, 0.0_f64);
    for f in 0..frames {
        #[cfg(feature = "log")]
        if let Some((from, to)) = log_range {
            LOG_ON.store(f >= from && f < to, Ordering::Relaxed);
        }
        if let Some((mask, from, to)) = trace {
            emu.config.trace_cpus = if f >= from && f < to { mask } else { 0 };
        }
        for (k, start, every, end) in &presses {
            if f > end.saturating_add(6) {
                continue;
            }
            if f >= *start && f <= *end && (f - start) % every == 0 {
                press(&mut emu, k, true);
            }
            if f >= *start + 6 && (f - start - 6) % every == 0 {
                press(&mut emu, k, false);
            }
        }
        for (a, v, at) in &pokes {
            if f == *at {
                emu.write_byte(*a, *v, ds_cpu::arm_cpu::CpuType::Arm9);
            }
        }
        let pen = touches.iter().find(|(_, _, start, every, end)| {
            f >= *start && f <= end.saturating_add(6) && (f - start) % every < 6
        });
        if let Some((x, y, ..)) = pen {
            emu.touchscreen_press(*x, *y);
        } else if gui_mode || !touches.is_empty() {
            // DS Touch Screen Controller (TSC): https://problemkaputt.de/gbatek.htm#dstouchscreencontrollertsc
            emu.touchscreen_press(0, 0xFFF); // Y = FFFh releases the pen
        }
        #[cfg(feature = "log")]
        if LOG_ON.load(Ordering::Relaxed) {
            eprintln!("=== frame {f}");
        }
        let t0 = std::time::Instant::now();
        emu.run();
        let _t1 = std::time::Instant::now();
        if let Some((scale, xbrz)) = hd_bench {
            let mut px = Vec::new();
            let th = std::time::Instant::now();
            let job = emu.gpu.hd_frame();
            bench.2 += th.elapsed().as_secs_f64();
            drop(job);
            let t1 = std::time::Instant::now();
            emu.gpu.screen_hd(true, scale, xbrz, &mut px);
            emu.gpu.screen_hd(false, scale, xbrz, &mut px);
            bench.0 += (t1 - t0).as_secs_f64();
            bench.1 += t1.elapsed().as_secs_f64();
            if f % 300 == 299 {
                println!(
                    "bench f={f} run={:.2}ms hd={:.2}ms copy={:.3}ms",
                    bench.0 / 300.0 * 1e3,
                    bench.1 / 300.0 * 1e3,
                    bench.2 / 300.0 * 1e3
                );
                bench = (0.0, 0.0, 0.0);
            }
        }
        if gui_mode {
            emu.take_audio_samples(&mut audio_buf);
            if wav_out.is_some() {
                wav_all.extend_from_slice(&audio_buf);
            }
            audio_buf.clear();
        }
        emu.get_upper_frame(&mut up);
        emu.get_lower_frame(&mut lo);
        let mut h = 0xcbf29ce484222325_u64; // FNV-1a 64-bit offset basis (frame hash, not hardware)
        for &p in up.iter().chain(lo.iter()) {
            h = (h ^ u64::from(p)).wrapping_mul(0x100000001b3); // FNV-1a 64-bit prime
        }
        if h != last_hash {
            last_hash = h;
            println!("frame {f}: screen changed");
        }
        if pcs_every != 0 && f % pcs_every == 0 {
            println!(
                "frame {f}: pc9={:08X} pc7={:08X} halt9={} halt7={} cpsr9={:08X}",
                emu.arm9.get_pc(),
                emu.arm7.get_pc(),
                emu.arm9.halted,
                emu.arm7.halted,
                emu.arm9.cpsr.get()
            );
            let t: Vec<String> = (4..8)
                .map(|i| {
                    let t = &emu.nds_timing.timers[i];
                    format!("{}:{}", t.enabled as u8, t.counter)
                })
                .collect();
            println!(
                "  IE9={:08X} IF9={:08X} IME9={} IE7={:08X} IF7={:08X} tm9={}",
                emu.int9_reg.irq_enable,
                emu.int9_reg.irq_flags,
                emu.int9_reg.ime,
                emu.int7_reg.irq_enable,
                emu.int7_reg.irq_flags,
                t.join(" ")
            );
            println!(
                "  romctrl={:08X} bytes_left={} dma_active={}",
                emu.cart.get_romctrl(),
                emu.cart.bytes_left,
                emu.dma_active()
            );
            let e3 = &emu.gpu.engine_3d;
            println!(
                "  3d: fifo={} pipe={} swap={} cycles={} gxstat={:08X}",
                e3.gxfifo.len(),
                e3.gxpipe.len(),
                e3.swap_buffers,
                e3.cycles,
                e3.get_gxstat()
            );
        }
        if f % png_every == png_every - 1 {
            save_png(&format!("{out}/{f:05}_top.png"), &up);
            save_png(&format!("{out}/{f:05}_bot.png"), &lo);
            // HD=<scale>[:x] also writes the upscaled screens (x = xBRZ).
            if let Ok(hd) = std::env::var("HD") {
                let scale: usize = hd.split(':').next().unwrap().parse().unwrap();
                let xbrz = hd.ends_with(":x");
                emu.gpu.set_render_scale(scale);
                let mut px = Vec::new();
                for (name, upper) in [("top", true), ("bot", false)] {
                    let w = emu.gpu.screen_hd(upper, scale, xbrz, &mut px);
                    let mut rgb = Vec::with_capacity(px.len() * 3);
                    for &p in &px {
                        rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
                    }
                    let h = (px.len() / w) as u32;
                    image::save_buffer(
                        format!("{out}/{f:05}_{name}_hd.png"),
                        &rgb,
                        w as u32,
                        h,
                        image::ExtendedColorType::Rgb8,
                    )
                    .unwrap();
                }
            }
        }
    }

    if let Some(path) = &wav_out {
        let mut f = Vec::new();
        let data_len = (wav_all.len() * 2) as u32;
        f.extend_from_slice(b"RIFF");
        f.extend_from_slice(&(36 + data_len).to_le_bytes());
        f.extend_from_slice(b"WAVEfmt ");
        f.extend_from_slice(&16_u32.to_le_bytes());
        f.extend_from_slice(&1_u16.to_le_bytes());
        f.extend_from_slice(&2_u16.to_le_bytes());
        f.extend_from_slice(&32768_u32.to_le_bytes());
        f.extend_from_slice(&(32768_u32 * 4).to_le_bytes());
        f.extend_from_slice(&4_u16.to_le_bytes());
        f.extend_from_slice(&16_u16.to_le_bytes());
        f.extend_from_slice(b"data");
        f.extend_from_slice(&data_len.to_le_bytes());
        for v in &wav_all {
            f.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, f).unwrap();
    }
    if let Ok(d) = std::env::var("DUMP") {
        let p: Vec<&str> = d.split(':').collect();
        let a = u32::from_str_radix(p[0], 16).unwrap();
        let n: u32 = p[1].parse().unwrap();
        let v: Vec<String> = (0..n).map(|i| format!("{:02X}", emu.arm9_read_byte(a + i))).collect();
        println!("dump {a:08X}: {}", v.join(" "));
    }

    if std::env::var("HIST").is_ok() {
        for (n, cpu) in [("arm9", &emu.arm9), ("arm7", &emu.arm7)] {
            let v: Vec<String> = cpu.recent_pcs().map(|p| format!("{p:08X}")).collect();
            println!("{n} history: {}", v.join(" "));
        }
    }
}
