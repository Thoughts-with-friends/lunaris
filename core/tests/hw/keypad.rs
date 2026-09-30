//! Spec tests for `core/src/hw/keypad.rs` — KEYINPUT, KEYCNT and the ARM7
//! EXTKEYIN register.
//!
//! GBATEK "DS Keypad": <https://problemkaputt.de/gbatek.htm#dskeypad>
//! GBATEK "GBA Keypad Input": <https://problemkaputt.de/gbatek.htm#gbakeypadinput>

use super::*;
use crate::{
    hw::HW,
    test_support::{
        boot::io_machine,
        md::{Doc, L, R, hx},
        spec::Checks,
    },
};

const KEYINPUT: u32 = 0x0400_0130;
const KEYCNT: u32 = 0x0400_0132;
const EXTKEYIN: u32 = 0x0400_0136;

/// `(key, register, bit)` per GBATEK.
const KEYS: &[(Key, &str, u32, &str)] = &[
    (Key::A, "KEYINPUT", 0, "A"),
    (Key::B, "KEYINPUT", 1, "B"),
    (Key::Select, "KEYINPUT", 2, "SELECT"),
    (Key::Start, "KEYINPUT", 3, "START"),
    (Key::Right, "KEYINPUT", 4, "RIGHT"),
    (Key::Left, "KEYINPUT", 5, "LEFT"),
    (Key::Up, "KEYINPUT", 6, "UP"),
    (Key::Down, "KEYINPUT", 7, "DOWN"),
    (Key::R, "KEYINPUT", 8, "R"),
    (Key::L, "KEYINPUT", 9, "L"),
    (Key::X, "EXTKEYIN", 0, "X"),
    (Key::Y, "EXTKEYIN", 1, "Y"),
];

#[test]
fn every_key_clears_exactly_its_bit_while_held() {
    let mut nds = io_machine("keypad_bits");
    let hw = nds.hw_mut();
    for &(key, reg, bit, name) in KEYS {
        let addr = if reg == "KEYINPUT" { KEYINPUT } else { EXTKEYIN };
        let read = |hw: &mut HW| {
            if reg == "KEYINPUT" { hw.arm9_read::<u16>(addr) } else { hw.arm7_read::<u16>(addr) }
        };
        let idle = read(hw);
        hw.press_key(key);
        assert_eq!(read(hw), idle & !(1 << bit), "{name} pressed (active low)");
        hw.release_key(key);
        assert_eq!(read(hw), idle, "{name} released");
    }
}

#[test]
fn keycnt_or_and_modes() {
    let mut nds = io_machine("keypad_irq");
    let hw = nds.hw_mut();
    // IRQ on A or B (OR mode).
    hw.arm9_write::<u16>(KEYCNT, 0x4003);
    assert!(!hw.keypad.interrupt_requested());
    hw.press_key(Key::B);
    assert!(hw.keypad.interrupt_requested(), "OR: any selected key");
    // AND mode needs both.
    hw.arm9_write::<u16>(KEYCNT, 0xC003);
    assert!(!hw.keypad.interrupt_requested(), "AND: B alone is not enough");
    hw.press_key(Key::A);
    assert!(hw.keypad.interrupt_requested(), "AND: A+B");
}

#[test]
fn report() {
    let mut nds = io_machine("keypad_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/keypad.md", "Keypad (KEYINPUT / KEYCNT / EXTKEYIN)", "hw/keypad.rs");
    doc.source("register-level test (no ROM data involved)");
    doc.p("All button bits are **active low**: 0 = pressed, 1 = released. The ten GBA-compatible buttons are visible to both CPUs; X, Y, the pen and the hinge are only visible to the ARM7 through EXTKEYIN.");
    doc.bitfield(
        "4000130h KEYINPUT (R)",
        16,
        &[
            (9, 9, "L"),
            (8, 8, "R"),
            (7, 7, "DN"),
            (6, 6, "UP"),
            (5, 5, "LT"),
            (4, 4, "RT"),
            (3, 3, "ST"),
            (2, 2, "SE"),
            (1, 1, "B"),
            (0, 0, "A"),
        ],
    );
    doc.bitfield(
        "4000132h KEYCNT (R/W)",
        16,
        &[(15, 15, "AND"), (14, 14, "IRQ"), (9, 0, "button mask (same layout as KEYINPUT)")],
    );
    doc.bitfield(
        "4000136h EXTKEYIN (ARM7, R)",
        16,
        &[
            (7, 7, "HINGE"),
            (6, 6, "PEN"),
            (5, 4, "1"),
            (3, 3, "DBG"),
            (2, 2, "1"),
            (1, 1, "Y"),
            (0, 0, "X"),
        ],
    );
    let mut c = Checks::new();
    let mut rows = Vec::new();
    for &(key, reg, bit, name) in KEYS {
        let read = |hw: &mut HW| {
            if reg == "KEYINPUT" {
                hw.arm9_read::<u16>(KEYINPUT)
            } else {
                hw.arm7_read::<u16>(EXTKEYIN)
            }
        };
        let idle = read(hw);
        hw.press_key(key);
        let held = read(hw);
        hw.release_key(key);
        c.eq(&format!("{name} → {reg} bit {bit}"), "0 while held", 0, (held >> bit) & 1);
        rows.push(vec![
            format!("`Key::{name}`"),
            reg.into(),
            bit.to_string(),
            hx(idle as u64, 4),
            hx(held as u64, 4),
        ]);
    }
    doc.h2("Key → register bit (observed)");
    doc.table(&[("Key", L), ("Register", L), ("Bit", R), ("Idle", R), ("Held", R)], &rows);
    c.hex("KEYINPUT idle", "all released = 03FFh", 0x03FFu16, hw.arm9_read::<u16>(KEYINPUT));
    c.hex(
        "EXTKEYIN idle",
        "X,Y,DBG released, pen up, hinge open; bits 2,4,5 read 1 → 007Fh",
        0x007Fu16,
        hw.arm7_read::<u16>(EXTKEYIN),
    );
    hw.press_screen(10, 10);
    c.eq(
        "pen down",
        "EXTKEYIN bit 6 = 0 while the screen is touched",
        0,
        hw.arm7_read::<u16>(EXTKEYIN) >> 6 & 1,
    );
    hw.release_screen();
    hw.arm9_write::<u16>(KEYINPUT, 0);
    c.hex("KEYINPUT read-only", "writes are ignored", 0x03FFu16, hw.arm9_read::<u16>(KEYINPUT));
    hw.arm9_write::<u16>(KEYCNT, 0xFFFF);
    c.hex("KEYCNT writable bits", "bits 0-9, 14, 15", 0xC3FFu16, hw.arm9_read::<u16>(KEYCNT));
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
