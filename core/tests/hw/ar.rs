//! Spec tests for `core/src/hw/ar.rs` — the Action Replay DS code
//! interpreter (desmume-compatible).
//!
//! Reference: "Action Replay DS code types" (Enhacklopedia / desmume
//! `CHEATS::ARparser`).

use super::*;
use crate::{
    ArCode,
    test_support::{
        boot::io_machine,
        md::{Doc, L},
        spec::Checks,
    },
};

const RAM: u32 = 0x0210_0000;

fn apply(hw: &mut HW, code: &[u32]) {
    hw.cheat_map = vec![ArCode { code: code.to_vec(), enabled: true }];
    hw.enable_cheats = true;
    hw.apply_cheats();
}

/// `(name, code pairs, check address, width, expected)`.
fn cases() -> Vec<(&'static str, Vec<u32>, u32, u32, u32)> {
    vec![
        ("0XXXXXXX: 32-bit write", vec![RAM, 0x1234_5678], RAM, 4, 0x1234_5678),
        ("1XXXXXXX: 16-bit write", vec![0x1000_0000 | RAM + 4, 0xABCD], RAM + 4, 2, 0xABCD),
        ("2XXXXXXX: 8-bit write", vec![0x2000_0000 | RAM + 6, 0xEF], RAM + 6, 1, 0xEF),
        (
            "5XXXXXXX: if equal → true",
            vec![RAM, 0x1234_5678, 0x5000_0000 | RAM, 0x1234_5678, RAM + 0x10, 1, 0xD200_0000, 0],
            RAM + 0x10,
            4,
            1,
        ),
        (
            "5XXXXXXX: if equal → false",
            vec![0x5000_0000 | RAM, 0xDEAD_DEAD, RAM + 0x14, 1, 0xD200_0000, 0],
            RAM + 0x14,
            4,
            0,
        ),
        (
            "3XXXXXXX: if greater (Y > [X])",
            vec![RAM + 0x18, 5, 0x3000_0000 | RAM + 0x18, 9, RAM + 0x1C, 7, 0xD200_0000, 0],
            RAM + 0x1C,
            4,
            7,
        ),
        (
            "D3: set offset, then offset write",
            vec![0xD300_0000, RAM + 0x20, 0x0000_0000, 0xAA, 0xD200_0000, 0],
            RAM + 0x20,
            4,
            0xAA,
        ),
        (
            "DC: add to offset",
            vec![0xD300_0000, RAM + 0x30, 0xDC00_0000, 4, 0x0000_0000, 0xBB, 0xD200_0000, 0],
            RAM + 0x34,
            4,
            0xBB,
        ),
        (
            "D5 + D6: store data, write word, offset += 4",
            vec![
                0xD300_0000,
                RAM + 0x40,
                0xD500_0000,
                0x77,
                0xD600_0000,
                0,
                0xD600_0000,
                0,
                0xD200_0000,
                0,
            ],
            RAM + 0x44,
            4,
            0x77,
        ),
        (
            "C0: loop (repeat Y+1 times)",
            vec![
                0xD300_0000,
                RAM + 0x50,
                0xC000_0000,
                3,
                0xD500_0000,
                0x11,
                0xD800_0000,
                0,
                0xD200_0000,
                0,
            ],
            RAM + 0x53,
            1,
            0x11,
        ),
    ]
}

fn read(hw: &mut HW, addr: u32, w: u32) -> u32 {
    match w {
        1 => hw.arm9_read::<u8>(addr) as u32,
        2 => hw.arm9_read::<u16>(addr) as u32,
        _ => hw.arm9_read::<u32>(addr),
    }
}

#[test]
fn code_types_write_what_the_reference_says() {
    for (name, code, addr, w, want) in cases() {
        let mut nds = io_machine("ar_cases");
        let hw = nds.hw_mut();
        apply(hw, &code);
        assert_eq!(read(hw, addr, w), want, "{name}");
    }
}

#[test]
fn report() {
    let mut doc = Doc::new("hw/ar.md", "Action Replay DS cheat interpreter", "hw/ar.rs");
    doc.source("codes applied to a fresh machine's main RAM through `HW::apply_cheats`");
    doc.p("A cheat is a flat list of `(hi, lo)` 32-bit pairs. The top nibble of `hi` is the code type (C and D are families selected by the whole top byte). Conditionals push onto a status stack; `D0` pops one level, `D2` resets everything (offset, data, stack). `apply_cheats` runs once per frame after `emulate_frame`.");
    doc.table(
        &[("Type", L), ("Form", L), ("Effect", L)],
        &[
            vec!["0", "0XXXXXXX YYYYYYYY", "[X + offset] = Y (32-bit)"],
            vec!["1", "1XXXXXXX 0000YYYY", "[X + offset] = Y (16-bit)"],
            vec!["2", "2XXXXXXX 000000YY", "[X + offset] = Y (8-bit)"],
            vec!["3–6", "3XXXXXXX YYYYYYYY", "if Y > / < / == / != [X] (32-bit)"],
            vec!["7–A", "7XXXXXXX ZZZZYYYY", "16-bit masked compares"],
            vec!["B", "BXXXXXXX 00000000", "offset = [X + offset]"],
            vec!["C0", "C0000000 YYYYYYYY", "loop: repeat block Y+1 times (until D1/D2)"],
            vec!["D0/D1/D2", "D?000000 00000000", "endif / next loop / next + full reset"],
            vec!["D3", "D3000000 XXXXXXXX", "offset = X"],
            vec!["D4", "D4000000 YYYYYYYY", "data += Y"],
            vec!["D5", "D5000000 YYYYYYYY", "data = Y"],
            vec!["D6/D7/D8", "D?000000 XXXXXXXX", "[X+offset] = data (32/16/8), offset += 4/2/1"],
            vec!["DC", "DC000000 YYYYYYYY", "offset += Y"],
            vec!["E", "EXXXXXXX YYYYYYYY", "copy Y bytes of immediate data to X + offset"],
        ],
    );
    let mut c = Checks::new();
    let mut rows = Vec::new();
    for (name, code, addr, w, want) in cases() {
        let mut nds = io_machine("ar_report");
        let hw = nds.hw_mut();
        apply(hw, &code);
        let got = read(hw, addr, w);
        c.eq(name, "reference semantics", format!("{want:X}"), format!("{got:X}"));
        let pairs: Vec<String> =
            code.chunks(2).map(|p| format!("`{:08X} {:08X}`", p[0], p[1])).collect();
        rows.push(vec![name.into(), pairs.join("<br>"), format!("{addr:08X}"), format!("{got:X}")]);
    }
    doc.h2("Executed codes");
    doc.table(&[("Case", L), ("Code", L), ("Checked address", L), ("Value after apply", L)], &rows);
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
