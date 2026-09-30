//! Spec tests for `core/src/hw/interrupt_controller.rs` — IME / IE / IF on
//! both CPUs.
//!
//! GBATEK "DS Interrupts": <https://problemkaputt.de/gbatek.htm#dsinterrupts>

use super::*;
use crate::test_support::{
    boot::io_machine,
    md::{Doc, L, R, hx},
    spec::Checks,
};

const IME: u32 = 0x0400_0208;
const IE: u32 = 0x0400_0210;
const IF: u32 = 0x0400_0214;

/// Every IE/IF bit: `(bit, name, source, arm9, arm7)`.
const SOURCES: &[(u32, &str, &str, bool, bool)] = &[
    (0, "VBLANK", "LCD V-Blank (DISPSTAT bit 3)", true, true),
    (1, "HBLANK", "LCD H-Blank (DISPSTAT bit 4)", true, true),
    (2, "VCOUNTER_MATCH", "VCOUNT == LYC (DISPSTAT bit 5)", true, true),
    (3, "TIMER0_OVERFLOW", "Timer 0 overflow", true, true),
    (4, "TIMER1_OVERFLOW", "Timer 1 overflow", true, true),
    (5, "TIMER2_OVERFLOW", "Timer 2 overflow", true, true),
    (6, "TIMER3_OVERFLOW", "Timer 3 overflow", true, true),
    (7, "SERIAL", "SIO/RCNT/RTC", false, true),
    (8, "DMA0", "DMA 0 end", true, true),
    (9, "DMA1", "DMA 1 end", true, true),
    (10, "DMA2", "DMA 2 end", true, true),
    (11, "DMA3", "DMA 3 end", true, true),
    (12, "KEYPAD", "KEYCNT condition", true, true),
    (13, "GAME_PAK", "GBA slot IREQ", true, true),
    (16, "IPC_SYNC", "IPCSYNC IRQ from the other CPU", true, true),
    (17, "IPC_SEND_FIFO_EMPTY", "own send FIFO became empty", true, true),
    (18, "IPC_RECV_FIFO_NOT_EMPTY", "receive FIFO got data", true, true),
    (19, "GAME_CARD_TRANSFER_COMPLETION", "NDS slot block transfer done", true, true),
    (20, "GAME_CARD_IREQ_MC", "NDS slot IREQ_MC", true, true),
    (21, "GEOMETRY_COMMAND_FIFO", "GXFIFO below half / empty", true, false),
    (22, "LID_OPEN", "hinge opened", false, true),
    (23, "SPI_BUS", "SPI transfer done", false, true),
    (24, "WIFI", "Wi-Fi", false, true),
];

#[test]
fn if_bits_are_write_one_to_clear() {
    let mut nds = io_machine("irq_if");
    let hw = nds.hw_mut();
    hw.interrupts[1].request = InterruptRequest::from_bits_truncate(0b1011);
    hw.arm9_write::<u32>(IF, 0b0010);
    assert_eq!(hw.arm9_read::<u32>(IF), 0b1001, "only the written 1 clears");
    hw.arm9_write::<u32>(IF, 0);
    assert_eq!(hw.arm9_read::<u32>(IF), 0b1001, "writing 0 leaves IF unchanged");
}

#[test]
fn irq_line_needs_ime_and_ie_and_if() {
    let mut nds = io_machine("irq_line");
    let hw = nds.hw_mut();
    hw.interrupts[1].request = InterruptRequest::VBLANK;
    hw.arm9_write::<u32>(IE, 0);
    hw.arm9_write::<u32>(IME, 1);
    assert!(!hw.arm9_interrupts_requested(), "IE masks the request");
    hw.arm9_write::<u32>(IE, 1);
    assert!(hw.arm9_interrupts_requested());
    hw.arm9_write::<u32>(IME, 0);
    assert!(!hw.arm9_interrupts_requested(), "IME=0 masks everything");
}

#[test]
fn ie_is_fully_read_write_on_both_cpus() {
    let mut nds = io_machine("irq_ie");
    let hw = nds.hw_mut();
    hw.arm9_write::<u32>(IE, 0x01FF_3FFF);
    assert_eq!(hw.arm9_read::<u32>(IE), 0x01FF_3FFF);
    hw.arm7_write::<u32>(IE, 0x0000_0101);
    assert_eq!(hw.arm7_read::<u32>(IE), 0x0000_0101);
    assert_eq!(hw.arm9_read::<u32>(IE), 0x01FF_3FFF, "the two controllers are independent");
}

#[test]
fn report() {
    let mut nds = io_machine("irq_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new(
        "hw/interrupt_controller.md",
        "Interrupt controller (IME / IE / IF)",
        "hw/interrupt_controller.rs",
    );
    doc.source("register-level test (no ROM data involved)");
    doc.p("Each CPU owns one controller (`HW::interrupts[0]` = ARM7, `[1]` = ARM9). A CPU takes an IRQ when `IME.0 && (IE & IF) != 0` and CPSR.I is clear.");
    doc.code(
        "text",
        "device ──set bit──► IF (4000214h) ──AND──► any bit? ──AND──► IRQ line ──► CPU (if CPSR.I = 0)\n\
         \x20                     ▲              ▲                     ▲\n\
         \x20   CPU writes 1 ─────┘ (clears)     IE (4000210h)          IME bit 0 (4000208h)",
    );
    doc.table(
        &[("Address", L), ("Name", L), ("Width", R), ("Semantics", L), ("Lunaris type", L)],
        &[
            vec!["4000208h", "IME", "32", "bit 0 = master enable", "`InterruptMasterEnable`"],
            vec!["4000210h", "IE", "32", "1 = source enabled", "`InterruptEnable`"],
            vec![
                "4000214h",
                "IF",
                "32",
                "1 = pending; **write 1 to acknowledge**",
                "`InterruptRequest`",
            ],
        ],
    );
    doc.h2("Interrupt sources");
    let rows: Vec<Vec<String>> = SOURCES
        .iter()
        .map(|&(bit, name, src, a9, a7)| {
            vec![
                bit.to_string(),
                hx(1u64 << bit, 7),
                format!("`{name}`"),
                src.into(),
                if a9 { "✓" } else { "" }.into(),
                if a7 { "✓" } else { "" }.into(),
            ]
        })
        .collect();
    doc.table(
        &[("Bit", R), ("Mask", R), ("Name", L), ("Source", L), ("ARM9", L), ("ARM7", L)],
        &rows,
    );
    doc.bitfield(
        "IE / IF bit layout",
        32,
        &[
            (24, 24, "WIFI"),
            (23, 23, "SPI"),
            (22, 22, "LID"),
            (21, 21, "GX"),
            (20, 20, "MC"),
            (19, 19, "CARD"),
            (18, 18, "RNE"),
            (17, 17, "SE"),
            (16, 16, "SYNC"),
            (13, 13, "PAK"),
            (12, 12, "KEY"),
            (11, 8, "DMA3-0"),
            (7, 7, "SIO"),
            (6, 3, "TMR3-0"),
            (2, 2, "VC"),
            (1, 1, "HB"),
            (0, 0, "VB"),
        ],
    );

    let mut c = Checks::new();
    for &(bit, name, ..) in SOURCES {
        c.hex(
            &format!("{name} constant"),
            &format!("IF bit {bit}"),
            1u32 << bit,
            InterruptRequest::from_bits_truncate(1 << bit).bits(),
        );
    }
    hw.interrupts[1].request = InterruptRequest::all();
    hw.arm9_write::<u32>(IF, InterruptRequest::VBLANK.bits());
    c.eq(
        "IF acknowledge",
        "writing 1 clears exactly that bit",
        false,
        hw.arm9_read::<u32>(IF) & 1 != 0,
    );
    hw.arm9_write::<u32>(IME, 0xFFFF_FFFF);
    c.known(
        "IME width",
        "only bit 0 of IME exists (reads back 1)",
        1,
        hw.arm9_read::<u32>(IME),
        "IME keeps every IE-valid bit instead of only bit 0",
    );
    hw.interrupts[0].request = InterruptRequest::empty();
    hw.interrupts[1].request = InterruptRequest::empty();
    hw.arm9_write::<u32>(IE, 1);
    hw.arm9_write::<u32>(IME, 1);
    hw.interrupts[1].request = InterruptRequest::VBLANK;
    c.eq("IME && IE && IF", "IRQ asserted", true, hw.arm9_interrupts_requested());
    c.eq(
        "ARM7 independent",
        "ARM9 IF does not assert ARM7 IRQ",
        false,
        hw.arm7_interrupts_requested(),
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
