//! Spec tests for `core/src/hw/timers.rs` — the 4+4 hardware timers,
//! prescalers, reload, IRQ and count-up (cascade) mode.
//!
//! GBATEK "DS Timers": <https://problemkaputt.de/gbatek.htm#dstimers>

use super::*;
use crate::test_support::{
    boot::io_machine,
    md::{Doc, L, R},
    spec::Checks,
};

const TM0: u32 = 0x0400_0100;

fn tm(i: u32) -> u32 {
    TM0 + i * 4
}

/// Starts ARM9 timer `i`: TMxCNT_L = reload, TMxCNT_H = `cnt`.
fn start(hw: &mut HW, i: u32, reload: u16, cnt: u16) {
    hw.arm9_write::<u32>(tm(i), reload as u32 | (cnt as u32) << 16);
}

fn counter(hw: &mut HW, i: u32) -> u16 {
    hw.arm9_read::<u16>(tm(i))
}

/// Advances time event by event (never skipping over a due event), like
/// `NDS::emulate_frame` does with its ≤30-cycle slices.
fn advance(hw: &mut HW, cycles: usize) {
    let target = hw.cycle() + cycles;
    while hw.cycle() < target {
        let next = hw.cycle_at_next_event().clamp(hw.cycle() + 1, target);
        hw.clock_until(next);
    }
}

#[test]
fn prescalers_divide_the_33mhz_clock() {
    for (sel, div) in [(0u16, 1usize), (1, 64), (2, 256), (3, 1024)] {
        let mut nds = io_machine(&format!("timer_ps{sel}"));
        let hw = nds.hw_mut();
        start(hw, 0, 0, 0x80 | sel);
        let c0 = counter(hw, 0);
        advance(hw, div * 100);
        let c1 = counter(hw, 0);
        let ticks = c1.wrapping_sub(c0) as usize;
        assert!((99..=101).contains(&ticks), "F/{div}: {ticks} ticks in {} cycles", div * 100);
    }
}

#[test]
fn overflow_reloads_and_raises_irq() {
    let mut nds = io_machine("timer_overflow");
    let hw = nds.hw_mut();
    start(hw, 0, 0xFFF0, 0x80 | 0x40);
    advance(hw, 0x20);
    assert!(hw.interrupts[1].request.contains(InterruptRequest::TIMER0_OVERFLOW));
    let c = counter(hw, 0);
    assert!(c >= 0xFFF0, "counter restarted from the reload value, got {c:#X}");
}

#[test]
fn count_up_timer_ticks_once_per_lower_overflow() {
    let mut nds = io_machine("timer_cascade");
    let hw = nds.hw_mut();
    start(hw, 1, 0, 0x80 | 0x04); // count-up
    start(hw, 0, 0xFF00, 0x80); // overflows every 100h cycles
    advance(hw, 0x100 * 5 + 0x10);
    assert_eq!(counter(hw, 1), 5);
}

#[test]
fn report() {
    let mut nds = io_machine("timer_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/timers.md", "Timers", "hw/timers.rs");
    doc.source("register-level test (no ROM data involved)");
    doc.p("Each CPU has four 16-bit up-counters clocked from the 33.513982 MHz bus clock (the ARM9's timers also run at 33 MHz, not 67 MHz). Lunaris does not tick them: it computes the counter from the scheduler cycle and schedules one `TimerOverflow` event per period.");
    doc.table(
        &[("Address (ARM9 / ARM7)", L), ("Name", L), ("Width", R), ("Read", L), ("Write", L)],
        &[
            vec!["4000100h + 4·n", "TMnCNT_L", "16", "current counter", "reload value"],
            vec!["4000102h + 4·n", "TMnCNT_H", "16", "control", "control"],
        ],
    );
    doc.bitfield(
        "TMnCNT_H",
        16,
        &[(7, 7, "START"), (6, 6, "IRQ"), (2, 2, "CNT-UP"), (1, 0, "PRESCALER")],
    );
    doc.code(
        "text",
        "counter(t) = reload + 1 + (t - start - first_tick) / prescaler      (Timer::calc_counter)\n\
         overflow at start + first_tick + prescaler·(10000h - reload - 1)    (Timer::create_event)\n\
         \n\
         \x20TM0 ──overflow──► TM1 (count-up) ──overflow──► TM2 (count-up) ──► TM3\n\
         \x20  │                 │                            │                  │\n\
         \x20  └── IF bit 3      └── IF bit 4                 └── IF bit 5       └── IF bit 6",
    );
    let mut c = Checks::new();
    let mut rows = Vec::new();
    for (sel, div) in [(0u16, 1usize), (1, 64), (2, 256), (3, 1024)] {
        let mut n = io_machine(&format!("timer_report_ps{sel}"));
        let h = n.hw_mut();
        start(h, 0, 0, 0x80 | sel);
        let c0 = counter(h, 0);
        advance(h, div * 1000);
        let ticks = counter(h, 0).wrapping_sub(c0);
        let freq = crate::NDS::CLOCK_RATE as f64 / div as f64;
        rows.push(vec![
            sel.to_string(),
            format!("F/{div}"),
            format!("{:.3} kHz", freq / 1000.0),
            format!("{:.2} µs", 1e6 / freq),
            ticks.to_string(),
        ]);
        c.ok(
            &format!("prescaler F/{div}"),
            "1000 periods → ≈1000 ticks",
            (999..=1001).contains(&ticks),
            ticks.to_string(),
        );
    }
    doc.h2("Prescalers (measured over 1000 periods)");
    doc.table(
        &[("Bits 0-1", R), ("Divider", L), ("Frequency", R), ("Tick", R), ("Ticks counted", R)],
        &rows,
    );

    start(hw, 0, 0xFFF0, 0xC0);
    advance(hw, 0x20);
    c.eq(
        "overflow IRQ",
        "IRQ bit 6 set → IF bit 3 on overflow",
        true,
        hw.interrupts[1].request.contains(InterruptRequest::TIMER0_OVERFLOW),
    );
    hw.arm9_write::<u16>(tm(0) + 2, 0);
    let mut n = io_machine("timer_report_cascade");
    let h = n.hw_mut();
    start(h, 1, 0xFFFE, 0x80 | 0x40 | 0x04);
    start(h, 0, 0xFF00, 0x80);
    advance(h, 0x100 * 2 + 0x10);
    c.hex(
        "cascade",
        "count-up TM1: FFFEh → FFFFh → overflow → reload FFFEh after 2 TM0 overflows",
        0xFFFEu16,
        counter(h, 1),
    );
    c.eq(
        "cascade IRQ",
        "TM1 overflow after 2 TM0 overflows → IF bit 4",
        true,
        h.interrupts[1].request.contains(InterruptRequest::TIMER1_OVERFLOW),
    );
    // Coarse clocking: jump 5 periods in one `clock_until`.
    let mut n = io_machine("timer_report_coarse");
    let h = n.hw_mut();
    start(h, 1, 0, 0x80 | 0x04);
    start(h, 0, 0xFF00, 0x80);
    let t = h.cycle() + 0x100 * 5 + 0x10;
    h.clock_until(t);
    doc.note(&format!(
        "API contract: `clock_until` must not jump past a due event. `NDS::emulate_frame` guarantees this          (`target = min(cycle + 30, cycle_at_next_event())`), so overflows are re-armed at their exact due          cycle. A single `clock_until` over 5 periods (as a test harness might do) re-arms from the jump          target instead and counts only {} overflow(s) — that is misuse of the API, not an emulation bug.",
        counter(h, 1)
    ));
    let mut n = io_machine("timer_report_stop");
    let h = n.hw_mut();
    start(h, 0, 0x1234, 0x80);
    advance(h, 100);
    h.arm9_write::<u16>(tm(0) + 2, 0);
    let frozen = counter(h, 0);
    advance(h, 1000);
    c.eq("stop freezes", "clearing START freezes the counter", frozen, counter(h, 0));
    h.arm9_write::<u16>(tm(2) + 2, 0xFF47);
    c.hex(
        "CNT_H read-back",
        "bits 0-2, 6, 7 readable; others read 0",
        0x47u16,
        h.arm9_read::<u16>(tm(2) + 2),
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
