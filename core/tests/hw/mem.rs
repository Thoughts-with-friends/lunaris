//! Spec tests for `core/src/hw/mem.rs` (+ `mem/arm9.rs`, `mem/arm7.rs`) —
//! the two memory maps, mirroring and the shared-WRAM split (WRAMCNT).
//!
//! GBATEK:
//! - "DS Memory Maps": <https://problemkaputt.de/gbatek.htm#dsmemorymaps>
//! - "DS Memory Control - WRAM": <https://problemkaputt.de/gbatek.htm#dsmemorycontrolwram>

use super::*;
use crate::test_support::{
    boot::io_machine,
    md::{Doc, L, R, hx, size},
    spec::Checks,
};

#[derive(Clone, Copy)]
enum Cpu {
    Arm9,
    Arm7,
}

fn w(hw: &mut HW, cpu: Cpu, a: u32, v: u32) {
    match cpu {
        Cpu::Arm9 => hw.arm9_write::<u32>(a, v),
        Cpu::Arm7 => hw.arm7_write::<u32>(a, v),
    }
}

fn r(hw: &mut HW, cpu: Cpu, a: u32) -> u32 {
    match cpu {
        Cpu::Arm9 => hw.arm9_read::<u32>(a),
        Cpu::Arm7 => hw.arm7_read::<u32>(a),
    }
}

/// `(cpu, name, base, size, mirror_at, note)`: a marker written at `base`
/// must read back at `mirror_at`.
const REGIONS: &[(Cpu, &str, u32, u32, u32, &str)] = &[
    (Cpu::Arm9, "Main RAM", 0x0200_0000, 0x40_0000, 0x0240_0000, "4 MiB, mirrored up to 2FFFFFFh"),
    (
        Cpu::Arm9,
        "Palette (engine A BG)",
        0x0500_0000,
        0x400,
        0x0500_0800,
        "2 KiB block mirrored to 5FFFFFFh",
    ),
    (
        Cpu::Arm9,
        "OAM (engine A)",
        0x0700_0000,
        0x400,
        0x0700_0800,
        "2 KiB block mirrored to 7FFFFFFh",
    ),
    (Cpu::Arm7, "Main RAM", 0x0200_0000, 0x40_0000, 0x0240_0000, "shared with the ARM9"),
    (Cpu::Arm7, "ARM7 WRAM", 0x0380_0000, 0x1_0000, 0x0381_0000, "64 KiB, mirrored to 3FFFFFFh"),
];

#[test]
fn main_ram_is_shared_between_cpus() {
    let mut nds = io_machine("mem_shared");
    let hw = nds.hw_mut();
    hw.arm9_write::<u32>(0x0230_0000, 0xCAFE_F00D);
    assert_eq!(hw.arm7_read::<u32>(0x0230_0000), 0xCAFE_F00D);
}

#[test]
fn wramcnt_splits_shared_wram_as_documented() {
    let mut nds = io_machine("mem_wramcnt");
    let hw = nds.hw_mut();
    hw.arm9_write::<u8>(0x0400_0247, 0);
    hw.arm9_write::<u32>(0x0300_0000, 0x1111_1111); // first 16 KiB
    hw.arm9_write::<u32>(0x0300_4000, 0x2222_2222); // second 16 KiB
    hw.arm9_write::<u8>(0x0400_0247, 1);
    assert_eq!(hw.arm9_read::<u32>(0x0300_0000), 0x2222_2222, "mode 1: ARM9 sees the 2nd half");
    assert_eq!(hw.arm7_read::<u32>(0x0300_0000), 0x1111_1111, "mode 1: ARM7 sees the 1st half");
    hw.arm9_write::<u8>(0x0400_0247, 2);
    assert_eq!(hw.arm9_read::<u32>(0x0300_0000), 0x1111_1111);
    assert_eq!(hw.arm7_read::<u32>(0x0300_0000), 0x2222_2222);
}

#[test]
fn report() {
    let mut nds = io_machine("mem_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/mem.md", "Memory maps, mirroring and WRAMCNT", "hw/mem.rs");
    doc.source("register-level probe through `HW::arm9_read/write` and `HW::arm7_read/write`");
    doc.p("Every bus access first looks up a 4 KiB page in a raw-pointer page table (`arm9_page_table` / `arm7_page_table`). RAM-like regions resolve there directly; everything else (I/O, VRAM, palettes, OAM, shared WRAM, slot-2) falls through to the per-region `match` in `mem/arm9.rs` / `mem/arm7.rs`.");
    doc.code(
        "text",
        "        ARM9 (ARM946E-S)                       ARM7 (ARM7TDMI)\n\
         00000000 ITCM 32K (CP15, mirrored)       00000000 BIOS7 16K\n\
         02000000 Main RAM 4M ─────────shared──── 02000000 Main RAM 4M\n\
         027FFE00   └ cartridge header copy\n\
         03000000 Shared WRAM 0/16/32K ◄WRAMCNT►  03000000 Shared WRAM 32/16/0K (else ARM7 WRAM)\n\
         \x20                                       03800000 ARM7 WRAM 64K\n\
         04000000 I/O (ARM9)                      04000000 I/O (ARM7), 04800000 Wi-Fi\n\
         05000000 Palettes 2K                     06000000 VRAM C/D as ARM7 WRAM\n\
         06000000 VRAM (engine A/B BG/OBJ, LCDC)\n\
         07000000 OAM 2K\n\
         08000000 GBA slot ROM / 0A000000 RAM\n\
         (DTCM 16K anywhere via CP15, default 00803000h in Lunaris)\n\
         FFFF0000 BIOS9 4K",
    );
    let mut c = Checks::new();
    let mut rows = Vec::new();
    for (i, &(cpu, name, base, sz, mirror, note)) in REGIONS.iter().enumerate() {
        let marker = 0x5A00_0000 | i as u32;
        w(hw, cpu, base, marker);
        let at_mirror = r(hw, cpu, mirror);
        let cpu_s = if matches!(cpu, Cpu::Arm9) { "ARM9" } else { "ARM7" };
        let item = format!("{cpu_s} {name} mirror");
        let rule = format!("{} reads back at {}", hx(base as u64, 8), hx(mirror as u64, 8));
        if matches!(base, 0x0500_0000 | 0x0700_0000) {
            c.known(&item, &rule, hx(marker as u64, 8), hx(at_mirror as u64, 8), "engine A/B is chosen with `addr & 7FFFh < 400h`, so the +800h mirror reaches engine B's block instead of wrapping every 2 KiB");
        } else {
            c.hex(&item, &rule, marker, at_mirror);
        }
        rows.push(vec![
            cpu_s.into(),
            name.into(),
            hx(base as u64, 8),
            size(sz as u64),
            hx(mirror as u64, 8),
            note.into(),
        ]);
    }
    doc.h2("Mirroring (probed)");
    doc.table(
        &[("CPU", L), ("Region", L), ("Base", R), ("Size", R), ("Mirror probed", R), ("GBATEK", L)],
        &rows,
    );

    let bios9: Vec<u32> = (0..4).map(|i| hw.arm9_read::<u32>(0xFFFF_0000 + i * 4)).collect();
    let bios9_ref: Vec<u32> = (0..4)
        .map(|i| {
            u32::from_le_bytes(free_bios::arm9::BIOS_ARM9_BIN[i * 4..i * 4 + 4].try_into().unwrap())
        })
        .collect();
    c.eq("BIOS9", "FFFF0000h = ARM9 BIOS (exception vectors)", bios9_ref, bios9);
    hw.arm9_write::<u32>(0xFFFF_0000, 0);
    c.hex(
        "BIOS9 read-only",
        "writes are ignored",
        u32::from_le_bytes(free_bios::arm9::BIOS_ARM9_BIN[..4].try_into().unwrap()),
        hw.arm9_read::<u32>(0xFFFF_0000),
    );

    doc.h2("WRAMCNT (4000247h) — shared WRAM split");
    doc.table(
        &[
            ("WRAMCNT", R),
            ("ARM9 03000000h", L),
            ("ARM7 03000000h", L),
            ("Lunaris (offset, mask) ARM9 / ARM7", L),
        ],
        &[
            vec!["0", "32K (whole)", "— (ARM7 WRAM mirror)", "(0, 7FFFh) / unmapped"],
            vec!["1", "16K: 2nd half", "16K: 1st half", "(4000h, 3FFFh) / (0, 3FFFh)"],
            vec!["2", "16K: 1st half", "16K: 2nd half", "(0, 3FFFh) / (4000h, 3FFFh)"],
            vec!["3", "— (undefined)", "32K (whole)", "unmapped / (0, 7FFFh)"],
        ],
    );
    hw.arm9_write::<u8>(0x0400_0247, 0);
    hw.arm9_write::<u32>(0x0300_0000, 0xAAAA_0001);
    hw.arm9_write::<u32>(0x0300_4000, 0xAAAA_0002);
    let mut wrows = Vec::new();
    for mode in 0..4u8 {
        hw.arm9_write::<u8>(0x0400_0247, mode);
        let a9 = hw.arm9_read::<u32>(0x0300_0000);
        let a7 = hw.arm7_read::<u32>(0x0300_0000);
        wrows.push(vec![mode.to_string(), hx(a9 as u64, 8), hx(a7 as u64, 8)]);
        let (e9, e7): (Option<u32>, Option<u32>) = match mode {
            0 => (Some(0xAAAA_0001), None),
            1 => (Some(0xAAAA_0002), Some(0xAAAA_0001)),
            2 => (Some(0xAAAA_0001), Some(0xAAAA_0002)),
            _ => (None, Some(0xAAAA_0001)),
        };
        if let Some(e) = e9 {
            c.hex(&format!("mode {mode} ARM9"), "ARM9 03000000h per table", e, a9);
        }
        if let Some(e) = e7 {
            c.hex(&format!("mode {mode} ARM7"), "ARM7 03000000h per table", e, a7);
        }
    }
    doc.p("Marker `AAAA0001` written at shared-WRAM offset 0, `AAAA0002` at offset 4000h (mode 0), then read at 03000000h in each mode:");
    doc.table(&[("WRAMCNT", R), ("ARM9 reads", R), ("ARM7 reads", R)], &wrows);
    hw.arm9_write::<u8>(0x0400_0247, 3);
    c.eq(
        "WRAMCNT read (ARM7)",
        "ARM7 sees WRAMSTAT at 4000241h",
        3,
        hw.arm7_read::<u8>(0x0400_0241),
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
