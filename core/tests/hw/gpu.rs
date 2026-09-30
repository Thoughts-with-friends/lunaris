//! Spec tests for `core/src/hw/gpu.rs` — LCD timing (VCOUNT, DISPSTAT,
//! H/V-Blank) and POWCNT1 screen routing.
//!
//! GBATEK:
//! - "DS Video Timings" / "LCD I/O Interrupts and Status":
//!   <https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus>
//! - "DS Power Control": <https://problemkaputt.de/gbatek.htm#dspowercontrol>

use super::*;
use crate::test_support::{
    boot::io_machine,
    md::{Doc, L, R, hx},
    spec::Checks,
};

const DISPSTAT: u32 = 0x0400_0004;
const VCOUNT: u32 = 0x0400_0006;

/// One observation per scheduler event: (cycle, vcount, dispstat).
fn trace_frame(hw: &mut HW, lines: usize) -> Vec<(usize, u16, u16)> {
    let mut out = Vec::new();
    let mut last = (u16::MAX, u16::MAX);
    let start_line = hw.arm9_read::<u16>(VCOUNT) as usize;
    let mut seen_lines = 0;
    while seen_lines < lines {
        let next = hw.cycle_at_next_event().max(hw.cycle() + 1);
        hw.clock_until(next);
        let (v, s) = (hw.arm9_read::<u16>(VCOUNT), hw.arm9_read::<u16>(DISPSTAT));
        if (v, s & 7) != last {
            if v != last.0 && v as usize != start_line {
                seen_lines += 1;
            }
            out.push((hw.cycle(), v, s));
            last = (v, s & 7);
        }
    }
    out
}

#[test]
fn a_line_is_355_dots_of_6_cycles_and_a_frame_263_lines() {
    let mut nds = io_machine("gpu_timing");
    let hw = nds.hw_mut();
    let t = trace_frame(hw, 264);
    let starts: Vec<(usize, u16)> = t.iter().filter(|e| e.2 & 2 == 0).map(|e| (e.0, e.1)).collect();
    let line_len: Vec<usize> =
        starts.windows(2).filter(|w| w[1].1 != w[0].1).map(|w| w[1].0 - w[0].0).collect();
    assert!(line_len.iter().skip(1).all(|&l| l == 355 * 6), "{line_len:?}");
    let max_v = t.iter().map(|e| e.1).max().unwrap();
    assert_eq!(max_v, 262, "VCOUNT runs 0..=262");
}

#[test]
fn report() {
    let mut nds = io_machine("gpu_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/gpu.md", "LCD timing, DISPSTAT and POWCNT1", "hw/gpu.rs");
    doc.source("register-level test: the scheduler is advanced event by event and VCOUNT/DISPSTAT are sampled after each event");
    doc.code(
        "text",
        "one scanline = 355 dots × 6 cycles (33.51 MHz) = 2130 cycles\n\
         ├───── 256 visible dots ─────┤·8·├── H-Blank flag set ──┤\n\
         0                          1536 1584 (dot 264)         2130\n\
         \x20                                  ▲ on_hblank: DISPSTAT.1 = 1, render_line(), HBlank DMA\n\
         ▲ start_next_line: VCOUNT += 1, DISPSTAT.1 = 0, VCOUNT-match check\n\
         \n\
         one frame = 263 lines = 560 190 cycles ≈ 59.83 Hz\n\
         line   0 ─ 191 : visible\n\
         line 192       : DISPSTAT.0 = 1, V-Blank IRQ, VBlank DMA, 3D render (Engine3D::render)\n\
         line 192 ─ 262 : V-Blank",
    );
    doc.bitfield(
        "4000004h DISPSTAT",
        16,
        &[
            (15, 7, "VCOUNT SETTING (bit 8 = bit 7 of reg)"),
            (5, 5, "VC IRQ"),
            (4, 4, "HB IRQ"),
            (3, 3, "VB IRQ"),
            (2, 2, "VC"),
            (1, 1, "HB"),
            (0, 0, "VB"),
        ],
    );
    hw.arm9_write::<u16>(DISPSTAT, 0x0038 | 100 << 8); // all IRQs, LYC = 100
    let t = trace_frame(hw, 264);
    let mut rows = Vec::new();
    let t0 = t[0].0;
    for e in &t {
        if matches!(e.1, 0 | 1 | 99 | 100 | 101 | 191 | 192 | 193 | 261 | 262) {
            rows.push(vec![
                (e.0 - t0).to_string(),
                e.1.to_string(),
                hx(e.2 as u64, 4),
                format!(
                    "{}{}{}",
                    if e.2 & 1 != 0 { "VB " } else { "" },
                    if e.2 & 2 != 0 { "HB " } else { "" },
                    if e.2 & 4 != 0 { "VC" } else { "" }
                ),
            ]);
        }
    }
    doc.h2("Sampled timeline (selected lines)");
    doc.table(&[("Cycle (rel.)", R), ("VCOUNT", R), ("DISPSTAT", R), ("Flags", L)], &rows);

    let mut c = Checks::new();
    let line_start = |v: u16| t.iter().find(|e| e.1 == v && e.2 & 2 == 0).map(|e| e.0);
    let hb = |v: u16| t.iter().find(|e| e.1 == v && e.2 & 2 != 0).map(|e| e.0);
    let (s10, s11) = (line_start(10).unwrap(), line_start(11).unwrap());
    c.eq("line length", "355 dots × 6 = 2130 cycles", 2130, s11 - s10);
    c.eq(
        "H-Blank start",
        "flag set 8 dots after the 256 visible dots (`GPU::HBLANK_DOT` = 264 → 1584 cycles)",
        GPU::HBLANK_DOT * GPU::CYCLES_PER_DOT,
        hb(10).unwrap() - s10,
    );
    c.eq("lines per frame", "VCOUNT 0..=262", 262, t.iter().map(|e| e.1).max().unwrap());
    let vb = |v: u16| t.iter().filter(|e| e.1 == v).all(|e| e.2 & 1 != 0);
    c.eq("V-Blank flag", "set on lines 192..=261", true, (192..=261).all(vb));
    c.eq(
        "visible lines",
        "V-Blank flag clear on lines 0..=191",
        true,
        (0..=191).all(|v| t.iter().filter(|e| e.1 == v).all(|e| e.2 & 1 == 0)),
    );
    c.known(
        "line 262",
        "V-Blank flag is cleared on the last line (262)",
        false,
        vb(262),
        "`start_next_line` clears the flag when VCOUNT wraps to 0 instead of at 262",
    );
    c.eq("VCOUNT match", "DISPSTAT.2 set exactly on line LYC (100)", vec![100u16], {
        let mut v: Vec<u16> = t.iter().filter(|e| e.2 & 4 != 0).map(|e| e.1).collect();
        v.dedup();
        v
    });
    let irq = hw.interrupts[1].request;
    c.eq(
        "IRQs",
        "V-Blank, H-Blank and VCOUNT-match IRQs all requested (IF bits 0-2)",
        0b111,
        irq.bits() & 7,
    );
    hw.arm9_write::<u16>(VCOUNT, 5);
    c.ok(
        "VCOUNT read-only",
        "writes are ignored",
        hw.arm9_read::<u16>(VCOUNT) != 5 || t.is_empty(),
        "wrote 5",
    );
    doc.h2("POWCNT1 (4000304h)");
    doc.bitfield(
        "4000304h POWCNT1",
        16,
        &[
            (15, 15, "A TOP"),
            (9, 9, "2D B"),
            (3, 3, "3D GEOM"),
            (2, 2, "3D RENDER"),
            (1, 1, "2D A"),
            (0, 0, "LCD"),
        ],
    );
    hw.arm9_write::<u16>(0x0400_0304, 0x8203);
    let a_top = std::ptr::eq(hw.gpu.get_screens()[0], hw.gpu.engine_a.pixels());
    hw.arm9_write::<u16>(0x0400_0304, 0x0203);
    let b_top = std::ptr::eq(hw.gpu.get_screens()[0], hw.gpu.engine_b.pixels());
    c.eq(
        "display swap",
        "POWCNT1.15 = 1 → engine A on the top screen, 0 → engine B",
        (true, true),
        (a_top, b_top),
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
