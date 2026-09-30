//! Spec tests for `core/src/arm.rs` — the shared CPU core: the execution
//! harness used by the ARM/THUMB instruction suites, condition codes, the
//! pipeline-visible PC and the IRQ entry sequence.
//!
//! GBATEK:
//! - "ARM Condition Field": <https://problemkaputt.de/gbatek.htm#armconditionfield>
//! - "ARM CPU Exceptions": <https://problemkaputt.de/gbatek.htm#armcpuexceptions>

use super::*;
use crate::{
    NDS,
    test_support::{
        boot::io_machine,
        md::{C, Doc, L},
        spec::Checks,
    },
};

/// Where test programs are placed (main RAM, far from the synthetic ROM).
pub(crate) const CODE: u32 = 0x0210_0000;

/// A CPU that has executed a test program, plus the machine it ran on.
pub(crate) struct Run<const A9: bool> {
    pub cpu: ARM<A9>,
    pub nds: NDS,
}

impl<const A9: bool> Run<A9> {
    pub fn r(&self, i: u32) -> u32 {
        self.cpu.regs[i]
    }
    /// `NZCV` as a 4-bit number.
    pub fn nzcv(&self) -> u32 {
        self.cpu.regs.cpsr() >> 28
    }
    pub fn cpsr(&self) -> u32 {
        self.cpu.regs.cpsr()
    }
}

/// Places `code` at [`CODE`] (32-bit words for ARM, halfwords in the low 16
/// bits for THUMB), appends `b .`, sets registers/flags, and runs the CPU
/// for a fixed cycle budget (the trailing `b .` absorbs the remainder).
pub(crate) fn run<const A9: bool>(
    name: &str,
    code: &[u32],
    thumb: bool,
    init: &[(u32, u32)],
    nzcv: u32,
) -> Run<A9> {
    let mut nds = io_machine(name);
    let hw = nds.hw_mut();
    if thumb {
        for (i, &h) in code.iter().chain(&[0xE7FE]).enumerate() {
            hw.arm9_write::<u16>(CODE + i as u32 * 2, h as u16);
        }
    } else {
        for (i, &w) in code.iter().chain(&[0xEAFF_FFFE]).enumerate() {
            hw.arm9_write::<u32>(CODE + i as u32 * 4, w);
        }
    }
    let mut cpu = ARM::<A9>::new(hw, true);
    // Direct boot leaves the entry address in r1/r12; start from a clean slate.
    for r in 0..13 {
        cpu.regs[r] = 0;
    }
    for &(r, v) in init {
        cpu.regs[r] = v;
    }
    *cpu.regs.cpsr_mut() = cpu.regs.cpsr() & 0x0FFF_FFFF | nzcv << 28;
    cpu.regs[15] = CODE;
    if thumb {
        cpu.regs.set_t(true);
        cpu.fill_thumb_instr_buffer(hw);
    } else {
        cpu.fill_arm_instr_buffer(hw);
    }
    let target = cpu.cycle + 20_000;
    cpu.emulate(hw, target);
    Run { cpu, nds }
}

/// GBATEK condition table, used as the reference for `should_exec`.
fn cond_ref(cond: u32, nzcv: u32) -> Option<bool> {
    let (n, z, c, v) = (nzcv & 8 != 0, nzcv & 4 != 0, nzcv & 2 != 0, nzcv & 1 != 0);
    Some(match cond {
        0x0 => z,
        0x1 => !z,
        0x2 => c,
        0x3 => !c,
        0x4 => n,
        0x5 => !n,
        0x6 => v,
        0x7 => !v,
        0x8 => c && !z,
        0x9 => !c || z,
        0xA => n == v,
        0xB => n != v,
        0xC => !z && n == v,
        0xD => z || n != v,
        0xE => true,
        _ => return None,
    })
}

const CONDS: [&str; 16] = [
    "EQ", "NE", "CS", "CC", "MI", "PL", "VS", "VC", "HI", "LS", "GE", "LT", "GT", "LE", "AL", "NV",
];

fn exec_with(r: &mut Run<true>, cond: u32, nzcv: u32) -> bool {
    *r.cpu.regs.cpsr_mut() = r.cpu.regs.cpsr() & 0x0FFF_FFFF | nzcv << 28;
    r.cpu.should_exec(cond)
}

#[test]
fn condition_codes_match_gbatek_for_all_flag_combinations() {
    let mut r = run::<true>("arm_cond", &[], false, &[], 0);
    for nzcv in 0..16u32 {
        for cond in 0..15u32 {
            assert_eq!(
                Some(exec_with(&mut r, cond, nzcv)),
                cond_ref(cond, nzcv),
                "{} with NZCV={nzcv:04b}",
                CONDS[cond as usize]
            );
        }
    }
}

#[test]
fn pc_reads_as_instruction_address_plus_8_in_arm_and_plus_4_in_thumb() {
    // mov r0, pc
    let a = run::<true>("arm_pc", &[0xE1A0_000F], false, &[], 0);
    assert_eq!(a.r(0), CODE + 8);
    // THUMB: mov r0, pc (hi-register MOV, H2 = 1)
    let t = run::<true>("thumb_pc", &[0x4678], true, &[], 0);
    assert_eq!(t.r(0), CODE + 4);
}

#[test]
fn report() {
    let mut doc = Doc::new("arm.md", "CPU core: conditions, pipeline, IRQ entry", "arm.rs");
    doc.source(
        "programs executed on a real `ARM<true>` / `ARM<false>` (see `run()` in the test file)",
    );
    doc.p("Both cores share one generic implementation `ARM<const IS_ARM9: bool>`. Instructions are dispatched through lookup tables built once: ARM by bits 27-20 + 7-4 (4096 entries), THUMB by bits 15-8 (256 entries). Every ARM instruction first passes the condition check `condition_lut[(NZCV << 4) | cond]`.");
    doc.code(
        "text",
        "fetch/decode/execute pipeline as seen by software\n\
         \n\
         address:  X        X+4      X+8\n\
         \x20         execute  decode   fetch      ← instr_buffer[0], instr_buffer[1]\n\
         \x20 ARM:    reading R15 while executing X gives X+8\n\
         \x20 THUMB:  reading R15 while executing X gives X+4",
    );
    doc.bitfield(
        "CPSR",
        32,
        &[
            (31, 31, "N"),
            (30, 30, "Z"),
            (29, 29, "C"),
            (28, 28, "V"),
            (27, 27, "Q"),
            (7, 7, "I"),
            (6, 6, "F"),
            (5, 5, "T"),
            (4, 0, "MODE"),
        ],
    );

    let mut base = run::<true>("arm_cond_report", &[], false, &[], 0);
    let mut c = Checks::new();
    let mut rows = Vec::new();
    for cond in 0..15u32 {
        let mut row = vec![format!("{:X}h", cond), CONDS[cond as usize].to_string()];
        let mut all_ok = true;
        for nzcv in 0..16u32 {
            let got = exec_with(&mut base, cond, nzcv);
            all_ok &= Some(got) == cond_ref(cond, nzcv);
            row.push(if got { "●" } else { "·" }.into());
        }
        c.ok(
            &format!("cond {}", CONDS[cond as usize]),
            "matches GBATEK for all 16 NZCV",
            all_ok,
            "16/16",
        );
        rows.push(row);
    }
    let mut headers: Vec<(String, _)> = vec![("Code".into(), L), ("Cond".into(), L)];
    for nzcv in 0..16u32 {
        headers.push((format!("{nzcv:04b}"), C));
    }
    let hdr: Vec<(&str, _)> = headers.iter().map(|(s, a)| (s.as_str(), *a)).collect();
    doc.h2("Condition codes (● = executes) — columns are NZCV");
    doc.table(&hdr, &rows);

    let a = run::<true>("arm_pc_report", &[0xE1A0_000F], false, &[], 0);
    c.hex("ARM PC", "R15 = address + 8", CODE + 8, a.r(0));
    let t = run::<true>("thumb_pc_report", &[0x4678], true, &[], 0);
    c.hex("THUMB PC", "R15 = address + 4", CODE + 4, t.r(0));

    // IRQ entry: IME/IE/IF set, CPSR.I clear → vector 18h relative to the
    // exception base (FFFF0000h on the ARM9 with CP15 high vectors).
    let mut r = run::<true>("arm_irq", &[], false, &[], 0);
    let hw = r.nds.hw_mut();
    hw.arm9_write::<u32>(0x0400_0208, 1);
    hw.arm9_write::<u32>(0x0400_0210, 1 << 12); // IE: keypad
    hw.arm9_write::<u16>(0x0400_0132, 0x4001); // KEYCNT: IRQ on A
    hw.press_key(crate::hw::Key::A);
    r.cpu.regs.set_i(false);
    let old_pc = r.cpu.regs[15];
    r.cpu.handle_irq(hw);
    let base9 = hw.cp15.interrupt_base();
    // Between instructions R15 = (next instruction) + 4 (pipeline refilled).
    c.hex(
        "IRQ vector",
        "next instruction = exception base + 18h",
        base9 | 0x18,
        r.cpu.regs[15] - 4,
    );
    c.eq(
        "IRQ mode",
        "CPSR mode = IRQ (12h), I set, T clear",
        (0x12u32, true, false),
        (r.cpu.regs.cpsr() & 0x1F, r.cpu.regs.get_i(), r.cpu.regs.get_t()),
    );
    c.hex(
        "IRQ LR",
        "LR_irq = interrupted instruction + 4 (`SUBS PC, LR, #4` resumes it)",
        (old_pc - 4) + 4,
        r.cpu.regs.lr(),
    );
    doc.h2("IRQ entry");
    doc.table(
        &[("Step", L), ("Effect", L)],
        &[
            vec!["1", "SPSR_irq = CPSR"],
            vec!["2", "mode = IRQ, CPSR.I = 1, CPSR.T = 0"],
            vec!["3", "LR_irq = return address + 4 (`SUBS PC, LR, #4` returns)"],
            vec!["4", "PC = base + 18h (ARM9: FFFF0000h when CP15 high vectors; ARM7: 0)"],
        ],
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
