//! Spec tests for `core/src/hw/mem/cp15.rs` — the ARM9 system control
//! coprocessor: control register and the ITCM/DTCM windows.
//!
//! GBATEK "ARM CP15 Tightly Coupled Memory (TCM)":
//! <https://problemkaputt.de/gbatek.htm#armcp15tightlycoupledmemorytcm>

use crate::test_support::{
    boot::io_machine,
    md::{Doc, L, R, hx, size},
    spec::Checks,
};

#[test]
fn dtcm_moves_with_c9_c1_0() {
    let mut nds = io_machine("cp15_dtcm");
    let hw = nds.hw_mut();
    hw.cp15.write(9, 1, 0, 0x027C_000A); // base 027C0000h, 512 << 5 = 16 KiB
    hw.init_arm9_page_tables();
    assert_eq!(hw.cp15.dtcm_range(), 0x027C_0000..0x027C_4000);
    hw.arm9_write::<u32>(0x027C_0000, 0x1357_9BDF);
    assert_eq!(hw.arm9_read::<u32>(0x027C_0000), 0x1357_9BDF);
    assert_ne!(
        hw.arm7_read::<u32>(0x027C_0000),
        0x1357_9BDF,
        "DTCM shadows main RAM for the ARM9 only"
    );
}

#[test]
fn report() {
    let mut nds = io_machine("cp15_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/mem/cp15.md", "CP15: control register and TCM", "hw/mem/cp15.rs");
    doc.source("direct `CP15::read/write` calls (the `MRC/MCR p15` path) on a fresh machine");
    doc.bitfield(
        "C9,C1,0 DTCM region / C9,C1,1 ITCM region",
        32,
        &[(31, 12, "BASE (DTCM only; ITCM base fixed 0)"), (5, 1, "SIZE: 512 << N"), (0, 0, "-")],
    );
    doc.table(
        &[("Register", L), ("Meaning", L), ("Lunaris", L)],
        &[
            vec!["C0,C0,1", "cache type (read-only)", "0F0D2112h"],
            vec![
                "C1,C0,0",
                "control: MPU, caches, high vectors (bit 13), TCM enables",
                "`CP15::control`",
            ],
            vec!["C2/C3", "cachability / write-buffer bits", "stored"],
            vec!["C5/C6", "access permissions / protection regions", "stored"],
            vec!["C7", "cache commands (incl. wait-for-IRQ)", "halt"],
            vec!["C9,C1,0/1", "DTCM / ITCM window", "`TCMControl` -> page tables"],
        ],
    );
    let mut c = Checks::new();
    c.hex("cache type", "C0,C0,1 = 0F0D2112h", 0x0F0D_2112u32, hw.cp15.read(0, 0, 1));
    let itcm = hw.cp15.itcm_range();
    let dtcm = hw.cp15.dtcm_range();
    doc.table(
        &[("TCM", L), ("Physical", R), ("Window after reset", L)],
        &[
            vec![
                "ITCM".into(),
                size(0x8000),
                format!("{}..{}", hx(itcm.start as u64, 8), hx(itcm.end as u64, 8)),
            ],
            vec![
                "DTCM".into(),
                size(0x4000),
                format!("{}..{}", hx(dtcm.start as u64, 8), hx(dtcm.end as u64, 8)),
            ],
        ],
    );
    hw.cp15.write(9, 1, 1, 16 << 1); // ITCM virtual size 512 << 16 = 32 MiB
    hw.init_arm9_page_tables();
    hw.arm9_write::<u32>(0x0000_0000, 0x2468_ACE0);
    c.hex(
        "ITCM mirror",
        "with a 32 MiB window (C9,C1,1 = 20h) the 32 KiB ITCM repeats (0 -> 8000h)",
        0x2468_ACE0u32,
        hw.arm9_read::<u32>(0x0000_8000),
    );
    hw.cp15.write(9, 1, 0, 0x027C_000A);
    hw.init_arm9_page_tables();
    c.eq(
        "DTCM move",
        "base 027C0000h, size 512 << 5",
        0x027C_0000..0x027C_4000,
        hw.cp15.dtcm_range(),
    );
    hw.arm9_write::<u32>(0x027C_0010, 0x1122_3344);
    c.eq(
        "DTCM is ARM9-private",
        "ARM7 at the same address sees main RAM",
        true,
        hw.arm7_read::<u32>(0x027C_0010) != 0x1122_3344,
    );
    let ctl = hw.cp15.read(1, 0, 0);
    hw.cp15.write(1, 0, 0, ctl | 1 << 13);
    c.hex(
        "high vectors",
        "control bit 13 -> exception base FFFF0000h",
        0xFFFF_0000u32,
        hw.cp15.interrupt_base(),
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
