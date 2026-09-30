//! Spec tests for `core/src/arm/registers.rs` — CPSR/SPSR and the banked
//! register file.
//!
//! GBATEK "ARM CPU Register Set": <https://problemkaputt.de/gbatek.htm#armcpuregisterset>

use super::*;
use crate::test_support::{
    md::{Doc, L},
    spec::Checks,
};

fn fresh() -> RegValues {
    let mut r = RegValues::new::<true>();
    for i in 0..15 {
        r[i] = 0x100 + i;
    }
    r
}

#[test]
fn irq_mode_banks_r13_r14_and_saves_cpsr() {
    let mut r = fresh();
    let cpsr = r.cpsr();
    r.change_mode(Mode::IRQ);
    r[13] = 0xAAAA;
    r[14] = 0xBBBB;
    assert_eq!(r.spsr(), cpsr, "SPSR_irq = old CPSR");
    r.restore_cpsr();
    assert_eq!((r[13], r[14]), (0x10D, 0x10E), "user R13/R14 restored");
}

#[test]
fn report() {
    let mut doc =
        Doc::new("arm/registers.md", "Register file, modes and banking", "arm/registers.rs");
    doc.source("direct calls on `RegValues`");
    doc.code(
        "text",
        "           R0-R7   R8-R12      R13 (SP)  R14 (LR)  R15   SPSR\n\
         USR/SYS   shared  shared      R13       R14       PC    -\n\
         FIQ       shared  R8_fiq-12   R13_fiq   R14_fiq   PC    SPSR_fiq\n\
         IRQ       shared  shared      R13_irq   R14_irq   PC    SPSR_irq\n\
         SVC       shared  shared      R13_svc   R14_svc   PC    SPSR_svc\n\
         UND       shared  shared      R13_und   R14_und   PC    SPSR_und\n\
         (ABT is not modelled by Lunaris: unreachable!())",
    );
    doc.table(
        &[("Mode", L), ("CPSR[4:0]", L), ("Banked in `RegValues`", L)],
        &[
            vec!["USR", "10000b", "-"],
            vec!["FIQ", "10001b", "`fiq: [u32; 7]` (R8-R14)"],
            vec!["IRQ", "10010b", "`irq: [u32; 2]`"],
            vec!["SVC", "10011b", "`svc: [u32; 2]`"],
            vec!["ABT", "10111b", "(not modelled)"],
            vec!["UND", "11011b", "`und: [u32; 2]`"],
            vec!["SYS", "11111b", "- (shares USR)"],
        ],
    );
    let mut c = Checks::new();
    for (mode, first_banked) in
        [(Mode::FIQ, 8u32), (Mode::IRQ, 13), (Mode::SVC, 13), (Mode::UND, 13)]
    {
        let mut r = fresh();
        let before: Vec<u32> = (0..15).map(|i| r[i]).collect();
        r.change_mode(mode);
        for i in first_banked..15 {
            r[i] = 0xF000 + i;
        }
        // Leave like MSR CPSR_c does (`set_mode`); `change_mode` is the
        // exception-entry path and would write SPSR, which in SYS is CPSR.
        r.set_mode(Mode::SYS);
        let after: Vec<u32> = (0..15).map(|i| r[i]).collect();
        c.eq(
            &format!("{mode:?} banking"),
            &format!("R{first_banked}-R14 banked; USR copies untouched"),
            before,
            after,
        );
        r.set_mode(mode);
        c.eq(
            &format!("{mode:?} bank persists"),
            "re-entering the mode restores its banked R14",
            0xF00E,
            r[14],
        );
    }
    let mut r = fresh();
    r.change_mode(Mode::SVC);
    *r.cpsr_mut() |= 1 << 29;
    r.change_mode(Mode::IRQ);
    c.eq(
        "SPSR on entry",
        "SPSR_irq = CPSR at the time of the switch (C set, SVC)",
        (1u32, 0x13u32),
        (r.spsr() >> 29 & 1, r.spsr() & 0x1F),
    );
    r.restore_cpsr();
    c.eq("restore", "CPSR = SPSR returns to SVC", Mode::SVC, r.get_mode());
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
