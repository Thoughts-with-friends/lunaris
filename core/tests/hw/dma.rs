//! Spec tests for `core/src/hw/dma.rs` — the 4+4 DMA channels: address
//! control, unit size, start timing, IRQ and the ARM9 DMA fill registers.
//!
//! GBATEK "DS DMA Transfers": <https://problemkaputt.de/gbatek.htm#dsdmatransfers>

use super::*;
use crate::test_support::{
    boot::io_machine,
    md::{Doc, L, R, hx},
    spec::Checks,
};

fn ch(n: u32) -> u32 {
    0x0400_00B0 + n * 0xC
}

fn advance(hw: &mut HW, cycles: usize) {
    let target = hw.cycle() + cycles;
    while hw.cycle() < target {
        let next = hw.cycle_at_next_event().clamp(hw.cycle() + 1, target);
        hw.clock_until(next);
    }
}

/// Programs ARM9 channel `n` and runs the scheduler long enough for an
/// immediate transfer to finish.
fn dma9(hw: &mut HW, n: u32, src: u32, dst: u32, cnt: u32) {
    hw.arm9_write::<u32>(ch(n), src);
    hw.arm9_write::<u32>(ch(n) + 4, dst);
    hw.arm9_write::<u32>(ch(n) + 8, cnt);
    advance(hw, 64);
}

fn fill_src(hw: &mut HW, base: u32, n: u32) {
    for i in 0..n {
        hw.arm9_write::<u32>(base + i * 4, 0x1000_0000 + i);
    }
}

fn read_words(hw: &mut HW, base: u32, n: u32) -> Vec<u32> {
    (0..n).map(|i| hw.arm9_read::<u32>(base + i * 4)).collect()
}

const SRC: u32 = 0x0210_0000;
const DST: u32 = 0x0220_0000;
const EN: u32 = 1 << 31;
const W32: u32 = 1 << 26;

#[test]
fn immediate_32bit_copy_increments_both_addresses() {
    let mut nds = io_machine("dma_copy");
    let hw = nds.hw_mut();
    fill_src(hw, SRC, 8);
    dma9(hw, 0, SRC, DST, EN | W32 | 8);
    assert_eq!(read_words(hw, DST, 8), read_words(hw, SRC, 8));
    assert_eq!(hw.arm9_read::<u32>(ch(0) + 8) & EN, 0, "enable bit clears when done");
}

#[test]
fn vblank_timed_dma_waits_for_vblank() {
    let mut nds = io_machine("dma_vblank");
    let hw = nds.hw_mut();
    fill_src(hw, SRC, 4);
    dma9(hw, 1, SRC, DST + 0x100, EN | W32 | 1 << 27 | 4);
    assert_eq!(hw.arm9_read::<u32>(DST + 0x100), 0, "not yet");
    advance(hw, 355 * 6 * 263);
    assert_eq!(read_words(hw, DST + 0x100, 4), read_words(hw, SRC, 4));
}

#[test]
fn report() {
    let mut nds = io_machine("dma_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/dma.md", "DMA controllers", "hw/dma.rs");
    doc.source("register-level test (no ROM data involved)");
    doc.table(
        &[("Address (ARM9 & ARM7)", L), ("Name", L), ("Width", R)],
        &[
            vec!["40000B0h + 0Ch·n", "DMAnSAD (source)", "32"],
            vec!["40000B4h + 0Ch·n", "DMAnDAD (destination)", "32"],
            vec!["40000B8h + 0Ch·n", "DMAnCNT (count + control)", "32"],
            vec!["40000E0h + 4·n", "DMAnFILL (ARM9 only)", "32"],
        ],
    );
    doc.bitfield(
        "DMAnCNT (ARM9)",
        32,
        &[
            (31, 31, "EN"),
            (30, 30, "IRQ"),
            (29, 27, "START"),
            (26, 26, "32BIT"),
            (25, 25, "RPT"),
            (24, 23, "SRC CTL"),
            (22, 21, "DST CTL"),
            (20, 0, "WORD COUNT (0 = 200000h)"),
        ],
    );
    doc.table(
        &[("START", R), ("ARM9", L), ("ARM7 (bits 28-29)", L), ("Lunaris `Occasion`", L)],
        &[
            vec!["0", "immediately", "immediately", "`Immediate`"],
            vec!["1", "V-Blank", "V-Blank", "`VBlank`"],
            vec!["2", "H-Blank (visible lines)", "DS cartridge", "`HBlank` / `DSCartridge`"],
            vec!["3", "start of display", "Wi-Fi / GBA slot", "`StartOfDisplay` (not implemented)"],
            vec!["4", "main-memory display", "–", "`MainMemoryDisplay` (not implemented)"],
            vec!["5", "DS cartridge", "–", "`DSCartridge`"],
            vec!["6", "GBA cartridge", "–", "`GBACartridge` (not implemented)"],
            vec!["7", "geometry FIFO (< half full)", "–", "`GeometryCommandFIFO`"],
        ],
    );
    doc.table(
        &[("CTL", R), ("Address step", L)],
        &[
            vec!["0", "increment"],
            vec!["1", "decrement"],
            vec!["2", "fixed"],
            vec!["3", "increment, reload DAD on repeat (dest only)"],
        ],
    );

    let mut c = Checks::new();
    let mut rows = Vec::new();
    let cases: &[(&str, u32, u32, u32, Vec<u32>)] = &[
        (
            "32-bit inc/inc",
            SRC,
            DST,
            EN | W32 | 4,
            vec![0x1000_0000, 0x1000_0001, 0x1000_0002, 0x1000_0003],
        ),
        (
            "dest decrement",
            SRC,
            DST + 0x4C,
            EN | W32 | 1 << 21 | 4,
            vec![0x1000_0003, 0x1000_0002, 0x1000_0001, 0x1000_0000],
        ),
        ("src fixed", SRC, DST + 0x80, EN | W32 | 2 << 23 | 4, vec![0x1000_0000; 4]),
        ("dest fixed", SRC, DST + 0xC0, EN | W32 | 2 << 21 | 4, vec![0x1000_0003, 0, 0, 0]),
    ];
    for (i, (name, s, d, cnt, want)) in cases.iter().enumerate() {
        fill_src(hw, SRC, 8);
        for k in 0..4 {
            hw.arm9_write::<u32>(DST + i as u32 * 0x40 + k * 4, 0);
        }
        dma9(hw, 0, *s, *d, *cnt);
        let base = if *name == "dest decrement" { DST + 0x40 } else { *d };
        let got = read_words(hw, base, 4);
        c.eq(name, "DMAnCNT address control", want.clone(), got.clone());
        rows.push(vec![
            name.to_string(),
            hx(*s as u64, 8),
            hx(*d as u64, 8),
            hx(*cnt as u64, 8),
            got.iter().map(|w| format!("{w:08X}")).collect::<Vec<_>>().join(" "),
        ]);
    }
    hw.arm9_write::<u32>(0x0400_00E0, 0xDEAD_BEEF);
    dma9(hw, 0, 0x0400_00E0, DST + 0x200, EN | W32 | 2 << 23 | 4);
    c.eq(
        "DMA fill",
        "src = DMA0FILL (fixed) → pattern fill",
        vec![0xDEAD_BEEFu32; 4],
        read_words(hw, DST + 0x200, 4),
    );
    fill_src(hw, SRC, 2);
    dma9(hw, 0, SRC, DST + 0x300, EN | 3);
    c.eq(
        "16-bit units",
        "count 3 = three halfwords: word 1 gets only its low half",
        vec![0x1000_0000u32, 0x0000_0001],
        read_words(hw, DST + 0x300, 2),
    );
    hw.interrupts[1].request = InterruptRequest::empty();
    dma9(hw, 2, SRC, DST + 0x400, EN | 1 << 30 | W32 | 1);
    c.eq(
        "IRQ at end",
        "bit 30 → IF bit 10 (DMA2)",
        true,
        hw.interrupts[1].request.contains(InterruptRequest::DMA2),
    );
    c.eq(
        "enable auto-clear",
        "non-repeating DMA clears bit 31 when done",
        0,
        hw.arm9_read::<u32>(ch(2) + 8) >> 31,
    );
    doc.h2("Transfers (observed)");
    doc.table(&[("Case", L), ("SAD", R), ("DAD", R), ("CNT", R), ("Result (4 words)", L)], &rows);
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
