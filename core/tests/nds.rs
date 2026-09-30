//! Spec tests for `core/src/nds.rs` — the top-level frame loop.
//!
//! GBATEK "DS Technical Data": <https://problemkaputt.de/gbatek.htm#dstechnicaldata>

use super::*;
use crate::test_support::{
    boot::{boot, frame_budget, run_until},
    md::{Doc, L, R, hx},
    png::{Canvas, WHITE},
    rom::{self, layout},
};

/// ARM9 cycles per frame: 355 dots × 263 lines × 6 ARM9 cycles per dot.
const ARM9_CYCLES_PER_FRAME: usize = 355 * 263 * 6;

#[test]
fn synthetic_rom_boots_and_draws_vram_gradient() {
    let mut nds = boot(rom::synthetic(), "nds_synthetic");
    let (frames, ok) = run_until(&mut nds, 4, |_| false);
    assert!(!ok && frames == 4);

    // The synthetic ARM9 program writes pixel value i to LCDC bank A.
    let [top, _] = nds.get_screens();
    for (i, &px) in top.iter().enumerate().take(0xC000) {
        assert_eq!(px & 0x7FFF, i as u16 & 0x7FFF, "pixel {i}");
    }

    let mut doc = Doc::new("nds.md", "NDS frame loop (`nds.rs`)", "nds.rs");
    doc.source(&rom::synthetic().label());
    doc.p("`NDS::emulate_frame` interleaves the two CPUs in slices of at most 30 \
         bus cycles and stops when the GPU raises `rendered_frame` at VBlank.")
        .code(
            "text",
            "loop until gpu.rendered_frame:\n\
         \x20 target = min(cycle + 30, next_scheduler_event)\n\
         \x20 ┌──────────────┐  2×target   ┌──────────────┐  1×target  ┌───────────┐\n\
         \x20 │ ARM9 (66MHz) │───────────►│ ARM7 (33MHz) │──────────►│ scheduler │\n\
         \x20 └──────────────┘             └──────────────┘            └───────────┘\n\
         \x20 (if the 3D bus is stalled: skip the CPUs, jump to the next event)",
        );
    doc.h2("Timing constants").table(
        &[("Quantity", L), ("Value", R), ("Derivation", L)],
        &[
            vec![
                "Master clock (ARM7)".into(),
                format!("{} Hz", NDS::CLOCK_RATE),
                "GBATEK: 33.513982 MHz".to_string(),
            ],
            vec!["ARM9 clock".into(), format!("{} Hz", NDS::CLOCK_RATE * 2), "2 × master".into()],
            vec!["Dots per line".into(), "355".into(), "256 visible + 99 HBlank".into()],
            vec!["Lines per frame".into(), "263".into(), "192 visible + 71 VBlank".into()],
            vec![
                "ARM9 cycles / frame".into(),
                ARM9_CYCLES_PER_FRAME.to_string(),
                "355 × 263 × 6".into(),
            ],
            vec![
                "Frame rate".into(),
                format!("{:.4} Hz", NDS::CLOCK_RATE as f64 / (355.0 * 263.0 * 6.0) * 2.0),
                "clock / (355 × 263 × 6 / 2)".into(),
            ],
        ],
    );
    doc.h2("Synthetic ROM boot").p(&format!(
        "The synthetic ROM's ARM9 program (`core/tests/support/rom.rs`) is copied to \
         {} and started directly (direct boot skips the BIOS/firmware). After {frames} frames \
         the top screen shows LCDC bank A, where pixel *i* = `i & 0x7FFF`:",
        hx(layout::ARM9_RAM as u64, 8)
    ));
    let img = Canvas::from_bgr555(top, WIDTH, HEIGHT, 1);
    img.save("nds/synthetic_top.png");
    doc.image("synthetic top screen", "nds/synthetic_top.png");
    doc.save();
}

#[test]
fn real_rom_boot_timeline() {
    let rom = rom::test_rom();
    let mut nds = boot(rom, "nds_timeline");
    let checkpoints = [1usize, 60, 180, 300, 450, 600, 900];
    let budget = frame_budget(*checkpoints.last().unwrap());
    let mut doc = Doc::new("nds_timeline.md", "Boot timeline", "nds.rs");
    doc.source(&rom.label());
    doc.p("Screens captured at fixed frame numbers after a direct boot (top | bottom).");
    let mut rows = Vec::new();
    let start = std::time::Instant::now();
    let mut done = 0;
    for &cp in &checkpoints {
        if cp > budget {
            break;
        }
        run_until(&mut nds, cp - done, |_| false);
        done = cp;
        let [top, bottom] = nds.get_screens();
        let mut c = Canvas::new(WIDTH * 2 + 4, HEIGHT, WHITE);
        c.blit_bgr555(0, 0, top, WIDTH, HEIGHT, 1);
        c.blit_bgr555(WIDTH + 4, 0, bottom, WIDTH, HEIGHT, 1);
        let file = format!("nds/frame_{cp:04}.png");
        c.save(&file);
        let arm9_pc = nds.cpus().1.regs()[15];
        rows.push(vec![cp.to_string(), hx(arm9_pc as u64, 8), format!("![f{cp}]({file})")]);
    }
    let elapsed = start.elapsed().as_secs_f64();
    doc.table(&[("Frame", R), ("ARM9 PC", L), ("Top \\| Bottom", L)], &rows);
    doc.p(&format!(
        "Emulated {done} frames in {elapsed:.1}s ({:.1} fps in this build profile).",
        done as f64 / elapsed
    ));
    doc.save();
}

#[test]
fn savestate_round_trip_is_deterministic() {
    let rom = rom::test_rom();
    let mut nds = boot(rom, "nds_savestate");
    run_until(&mut nds, 120, |_| false);
    let state = nds.save_state().unwrap();
    run_until(&mut nds, 30, |_| false);
    let a: Vec<u16> = nds.get_screens().iter().flat_map(|s| s.iter().copied()).collect();
    nds.load_state(&state).unwrap();
    run_until(&mut nds, 30, |_| false);
    let b: Vec<u16> = nds.get_screens().iter().flat_map(|s| s.iter().copied()).collect();
    let mut doc = Doc::new("nds_savestate.md", "Savestate round trip", "nds.rs");
    doc.source(&rom.label());
    doc.p(&format!(
        "`NDS::save_state` after 120 frames produced {} bytes (the ROM, BIOS and firmware are not stored; see docs/ref-gbatek ch. 19).          Running 30 more frames, restoring the state and running the same 30 frames again must give identical screens.",
        state.len()
    ));
    let mut c = crate::test_support::spec::Checks::new();
    c.eq("deterministic replay", "frame 150 is identical after save → load → replay", true, a == b);
    // The ROM is not part of the state: a machine booted from the 128 KiB
    // synthetic ROM must produce a state of (nearly) the same size.
    let mut small = boot(rom::synthetic(), "nds_savestate_small");
    run_until(&mut small, 2, |_| false);
    let small_len = small.save_state().unwrap().len() - small.export_save().len();
    let big_len = state.len() - nds.export_save().len();
    c.ok(
        "ROM not stored",
        "state size minus the embedded save chip does not depend on the ROM size (±64 KiB)",
        big_len.abs_diff(small_len) < 0x1_0000,
        format!(
            "{big_len} bytes ({} B ROM) vs {small_len} bytes (128 KiB synthetic ROM)",
            rom.bytes.len()
        ),
    );
    c.write(&mut doc);
    doc.save();
    c.finish();
}
