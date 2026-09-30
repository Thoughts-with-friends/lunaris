//! Spec tests for `core/src/hw/math.rs` — the ARM9 divider and square-root
//! units, driven through their I/O ports exactly as game code does.
//!
//! GBATEK "DS Maths": <https://problemkaputt.de/gbatek.htm#dsmaths>

use crate::test_support::{
    boot::io_machine,
    md::{Doc, L, R},
    spec::Checks,
};

const DIVCNT: u32 = 0x0400_0280;
const DIV_NUMER: u32 = 0x0400_0290;
const DIV_DENOM: u32 = 0x0400_0298;
const DIV_RESULT: u32 = 0x0400_02A0;
const DIVREM_RESULT: u32 = 0x0400_02A8;
const SQRTCNT: u32 = 0x0400_02B0;
const SQRT_RESULT: u32 = 0x0400_02B4;
const SQRT_PARAM: u32 = 0x0400_02B8;

fn div(hw: &mut super::HW, mode: u16, numer: u64, denom: u64) -> (u64, u64, bool) {
    hw.arm9_write::<u16>(DIVCNT, mode);
    hw.arm9_write::<u32>(DIV_NUMER, numer as u32);
    hw.arm9_write::<u32>(DIV_NUMER + 4, (numer >> 32) as u32);
    hw.arm9_write::<u32>(DIV_DENOM, denom as u32);
    hw.arm9_write::<u32>(DIV_DENOM + 4, (denom >> 32) as u32);
    let r64 = |hw: &mut super::HW, a| {
        hw.arm9_read::<u32>(a) as u64 | (hw.arm9_read::<u32>(a + 4) as u64) << 32
    };
    let q = r64(hw, DIV_RESULT);
    let r = r64(hw, DIVREM_RESULT);
    let by0 = hw.arm9_read::<u16>(DIVCNT) & 0x4000 != 0;
    (q, r, by0)
}

fn sqrt(hw: &mut super::HW, mode64: bool, param: u64) -> u32 {
    hw.arm9_write::<u16>(SQRTCNT, mode64 as u16);
    hw.arm9_write::<u32>(SQRT_PARAM, param as u32);
    hw.arm9_write::<u32>(SQRT_PARAM + 4, (param >> 32) as u32);
    hw.arm9_read::<u32>(SQRT_RESULT)
}

/// `(mode, numer, denom, quot, rem, div0, description)` — every row is a
/// GBATEK statement.
const DIV_CASES: &[(u16, i64, i64, i64, i64, bool, &str)] = &[
    (0, 7, 2, 3, 1, false, "32/32: truncates toward zero"),
    (0, -7, 2, -3, -1, false, "32/32: remainder takes numerator sign"),
    (0, 7, -2, -3, 1, false, "32/32: negative divisor"),
    (1, 0x1_0000_0000, 2, 0x8000_0000, 0, false, "64/32: numerator uses all 64 bits"),
    (2, 0x7FFF_FFFF_FFFF_FFFF, 0x1_0000_0000, 0x7FFF_FFFF, 0xFFFF_FFFF, false, "64/64"),
    (1, 5, 0x1_0000_0003, 1, 2, false, "64/32: only the low 32 bits of DENOM are used"),
    (2, 5, 0, -1, 5, true, "÷0 (64/64): QUOT = -1 (sign of numer inverted), REM = numer"),
    (2, -5, 0, 1, -5, true, "÷0 negative numer: QUOT = +1"),
    (2, 0, 0, -1, 0, true, "0÷0: QUOT = -1"),
    (2, i64::MIN, -1, i64::MIN, 0, false, "overflow: MIN / -1 = MIN, REM 0"),
];

#[test]
fn divider_matches_gbatek_table() {
    let mut nds = io_machine("math_div");
    let hw = nds.hw_mut();
    for &(mode, n, d, q, r, by0, what) in DIV_CASES {
        let got = div(hw, mode, n as u64, d as u64);
        assert_eq!(got, (q as u64, r as u64, by0), "{what}");
    }
}

#[test]
fn divider_32bit_div0_inverts_upper_quotient_word() {
    // GBATEK: in 32/32 mode, ÷0 gives QUOT = ±1 with bits 32-63 inverted.
    let mut nds = io_machine("math_div0_32");
    let (q, r, by0) = div(nds.hw_mut(), 0, 5, 0);
    assert!(by0);
    assert_eq!(q, 0x0000_0000_FFFF_FFFF);
    assert_eq!(r, 5);
}

#[test]
fn sqrt_is_floor_of_exact_root() {
    let mut nds = io_machine("math_sqrt");
    let hw = nds.hw_mut();
    for v in [0u64, 1, 2, 3, 4, 15, 16, 17, 0xFFFF_FFFF] {
        assert_eq!(sqrt(hw, false, v) as u64, (v as f64).sqrt().floor() as u64, "32-bit sqrt({v})");
    }
    assert_eq!(sqrt(hw, true, u64::MAX), 0xFFFF_FFFF);
    assert_eq!(sqrt(hw, false, 0x1_0000_0010), 4, "32-bit mode ignores PARAM bits 32-63");
}

#[test]
fn report() {
    let mut nds = io_machine("math_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/math.md", "Maths units (DIV / SQRT)", "hw/math.rs");
    doc.source("register-level test on a freshly constructed machine (no ROM data involved)");
    doc.p("Both units are memory-mapped calculators on the ARM9 bus. Writing any byte of a parameter recomputes the result immediately in Lunaris (hardware takes 18–34 cycles for DIV and 13 for SQRT; the busy bit is never set).");
    doc.h2("Register map");
    doc.table(
        &[("Address", L), ("Name", L), ("Size", R), ("Access", L), ("Lunaris field", L)],
        &[
            vec![
                "4000280h",
                "DIVCNT",
                "4",
                "R/W",
                "`Div::cnt` (`DIVCNT { mode, div_by_0, busy }`)",
            ],
            vec!["4000290h", "DIV_NUMER", "8", "R/W", "`Div::numer`"],
            vec!["4000298h", "DIV_DENOM", "8", "R/W", "`Div::denom`"],
            vec!["40002A0h", "DIV_RESULT", "8", "R", "`Div::quot`"],
            vec!["40002A8h", "DIVREM_RESULT", "8", "R", "`Div::rem`"],
            vec!["40002B0h", "SQRTCNT", "4", "R/W", "`Sqrt::cnt` (`SQRTCNT { is_64bit, busy }`)"],
            vec!["40002B4h", "SQRT_RESULT", "4", "R", "`Sqrt::result`"],
            vec!["40002B8h", "SQRT_PARAM", "8", "R/W", "`Sqrt::param`"],
        ],
    );
    doc.bitfield("4000280h DIVCNT", 16, &[(15, 15, "BUSY"), (14, 14, "DIV0"), (1, 0, "MODE")]);
    doc.table(
        &[("MODE", L), ("Numerator", L), ("Denominator", L), ("Result", L)],
        &[
            vec!["0", "s32 (NUMER bits 0-31)", "s32", "s32 (sign-extended to 64)"],
            vec!["1", "s64", "s32", "s64"],
            vec!["2", "s64", "s64", "s64"],
            vec!["3", "reserved — behaves like 1 (used by *Kingdom Hearts 358/2 Days*)", "", ""],
        ],
    );
    doc.bitfield("40002B0h SQRTCNT", 16, &[(15, 15, "BUSY"), (0, 0, "64")]);
    doc.code(
        "text",
        "   NUMER (64) ─┐                         ┌─► DIV_RESULT    (quotient)\n\
         \x20              ├─► Div::calc() on write ─┤\n\
         \x20  DENOM (64) ─┘   mode from DIVCNT      └─► DIVREM_RESULT (remainder)\n\
         \n\
         \x20  SQRT_PARAM (32/64) ─► floor(sqrt(x)) ─► SQRT_RESULT (32)",
    );

    let mut c = Checks::new();
    let mut rows = Vec::new();
    for &(mode, n, d, q, r, by0, what) in DIV_CASES {
        let (gq, gr, gb) = div(hw, mode, n as u64, d as u64);
        c.eq(what, "GBATEK DS Maths", (q, r, by0), (gq as i64, gr as i64, gb));
        rows.push(vec![
            mode.to_string(),
            n.to_string(),
            d.to_string(),
            (gq as i64).to_string(),
            (gr as i64).to_string(),
            gb.to_string(),
        ]);
    }
    let (q, _, _) = div(hw, 0, 5, 0);
    c.hex("32/32 ÷0 QUOT", "±1 with upper 32 bits inverted", 0x0000_0000_FFFF_FFFFu64, q);
    hw.arm9_write::<u16>(DIVCNT, 3);
    c.eq(
        "DIVCNT mode 3 read-back",
        "mode field is 2 bits, R/W",
        3,
        hw.arm9_read::<u16>(DIVCNT) & 3,
    );
    // GBATEK: results are recomputed when DIVCNT changes, too.
    div(hw, 0, 0x1_0000_0004, 2);
    hw.arm9_write::<u16>(DIVCNT, 1);
    let q_after_mode = hw.arm9_read::<u32>(DIV_RESULT);
    c.known(
        "DIVCNT write recomputes",
        "writing DIVCNT restarts the division",
        0x8000_0002u32,
        q_after_mode,
        "`Div::calc` only runs on NUMER/DENOM writes",
    );
    for (m64, v) in [(false, 1_000_000u64), (true, 1u64 << 62), (false, 0xFFFF_FFFF)] {
        let got = sqrt(hw, m64, v);
        let expect = (v as f64).sqrt().floor() as u32;
        c.eq(&format!("sqrt({v}) {}-bit", if m64 { 64 } else { 32 }), "floor(√x)", expect, got);
    }
    doc.h2("Divider edge cases (observed through the I/O ports)");
    doc.table(
        &[("MODE", R), ("NUMER", R), ("DENOM", R), ("QUOT", R), ("REM", R), ("DIV0", L)],
        &rows,
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
