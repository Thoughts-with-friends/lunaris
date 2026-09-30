//! Spec tests for `core/src/arm/arm.rs` — the 32-bit ARM instruction set,
//! executed on both cores.
//!
//! GBATEK "ARM Instruction Set" (opcode formats):
//! <https://problemkaputt.de/gbatek.htm#arminstructionsummary>

use crate::{
    arm::spec::{CODE, Run, run},
    test_support::{
        md::{Doc, L},
        spec::Checks,
    },
};

// ---------------------------------------------------------------------------
// Tiny encoder (only what the cases need; cond = AL unless noted)
// ---------------------------------------------------------------------------

const AL: u32 = 0xE << 28;

fn dp_imm(op: u32, s: bool, rd: u32, rn: u32, imm8: u32, rot: u32) -> u32 {
    AL | 1 << 25 | op << 21 | (s as u32) << 20 | rn << 16 | rd << 12 | rot << 8 | imm8
}
fn dp_reg(op: u32, s: bool, rd: u32, rn: u32, rm: u32, sh_type: u32, sh_imm: u32) -> u32 {
    AL | op << 21 | (s as u32) << 20 | rn << 16 | rd << 12 | sh_imm << 7 | sh_type << 5 | rm
}
fn dp_rsr(op: u32, s: bool, rd: u32, rn: u32, rm: u32, sh_type: u32, rs: u32) -> u32 {
    AL | op << 21 | (s as u32) << 20 | rn << 16 | rd << 12 | rs << 8 | sh_type << 5 | 1 << 4 | rm
}
fn cond(instr: u32, c: u32) -> u32 {
    instr & 0x0FFF_FFFF | c << 28
}
const AND: u32 = 0x0;
const SUB: u32 = 0x2;
const RSB: u32 = 0x3;
const ADD: u32 = 0x4;
const ADC: u32 = 0x5;
const SBC: u32 = 0x6;
const CMP: u32 = 0xA;
const ORR: u32 = 0xC;
const MOV: u32 = 0xD;
const BIC: u32 = 0xE;
const MVN: u32 = 0xF;
const LSL: u32 = 0;
const LSR: u32 = 1;
const ASR: u32 = 2;
const ROR: u32 = 3;

fn mul(rd: u32, rm: u32, rs: u32) -> u32 {
    AL | 0x90 | rd << 16 | rs << 8 | rm
}
fn mla(rd: u32, rm: u32, rs: u32, rn: u32) -> u32 {
    AL | 0x0020_0090 | rd << 16 | rn << 12 | rs << 8 | rm
}
fn umull(lo: u32, hi: u32, rm: u32, rs: u32) -> u32 {
    AL | 0x0080_0090 | hi << 16 | lo << 12 | rs << 8 | rm
}
fn smull(lo: u32, hi: u32, rm: u32, rs: u32) -> u32 {
    AL | 0x00C0_0090 | hi << 16 | lo << 12 | rs << 8 | rm
}
fn ldr(rd: u32, rn: u32, off: u32) -> u32 {
    AL | 0x0590_0000 | rn << 16 | rd << 12 | off
}
fn str_(rd: u32, rn: u32, off: u32) -> u32 {
    AL | 0x0580_0000 | rn << 16 | rd << 12 | off
}
fn ldrb(rd: u32, rn: u32, off: u32) -> u32 {
    AL | 0x05D0_0000 | rn << 16 | rd << 12 | off
}
fn half(op: u32, rd: u32, rn: u32, off: u32) -> u32 {
    // op: B0 = LDRH/STRH, D0 = LDRSB, F0 = LDRSH; bit 20 = load
    AL | 0x01C0_0000 | rn << 16 | rd << 12 | (off >> 4) << 8 | op | off & 0xF
}
fn stmia_w(rn: u32, list: u32) -> u32 {
    AL | 0x08A0_0000 | rn << 16 | list
}
fn ldmia_w(rn: u32, list: u32) -> u32 {
    AL | 0x08B0_0000 | rn << 16 | list
}
fn b(off_words: i32) -> u32 {
    AL | 0x0A00_0000 | (off_words as u32 & 0xFF_FFFF)
}
fn bl(off_words: i32) -> u32 {
    AL | 0x0B00_0000 | (off_words as u32 & 0xFF_FFFF)
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

type Check = fn(&Run<true>) -> (u64, u64);

struct Case {
    name: &'static str,
    asm: &'static str,
    code: Vec<u32>,
    init: Vec<(u32, u32)>,
    nzcv: u32,
    arm9_only: bool,
    /// `(expected, actual)` extracted from the finished run.
    check: Check,
    rule: &'static str,
}

fn r(run: &Run<true>, i: u32) -> u64 {
    run.r(i) as u64
}

fn cases() -> Vec<Case> {
    let base = CODE + 0x100;
    vec![
        Case {
            name: "MOV imm",
            asm: "mov r0, #42h",
            code: vec![dp_imm(MOV, false, 0, 0, 0x42, 0)],
            init: vec![],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0x42, r(x, 0)),
            rule: "Rd = imm8 ROR (2·rot)",
        },
        Case {
            name: "MOV rotated imm",
            asm: "mov r1, #FF000000h",
            code: vec![dp_imm(MOV, false, 1, 0, 0xFF, 4)],
            init: vec![],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0xFF00_0000, r(x, 1)),
            rule: "imm8 = FFh, rot = 4 → ROR 8",
        },
        Case {
            name: "ADDS overflow",
            asm: "adds r1, r0, #1",
            code: vec![dp_imm(ADD, true, 1, 0, 1, 0)],
            init: vec![(0, 0x7FFF_FFFF)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0x8000_0000_9, r(x, 1) << 4 | x.nzcv() as u64),
            rule: "7FFFFFFFh + 1 = 80000000h, N=1 V=1",
        },
        Case {
            name: "SUBS zero",
            asm: "subs r1, r0, #5",
            code: vec![dp_imm(SUB, true, 1, 0, 5, 0)],
            init: vec![(0, 5)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0x0_6, r(x, 1) << 4 | x.nzcv() as u64),
            rule: "5 − 5 = 0, Z=1 C=1 (no borrow)",
        },
        Case {
            name: "SUBS borrow",
            asm: "subs r1, r0, #5",
            code: vec![dp_imm(SUB, true, 1, 0, 5, 0)],
            init: vec![(0, 3)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0xFFFF_FFFE_8, r(x, 1) << 4 | x.nzcv() as u64),
            rule: "3 − 5 = −2, N=1 C=0 (borrow)",
        },
        Case {
            name: "RSB",
            asm: "rsb r1, r0, #0",
            code: vec![dp_imm(RSB, false, 1, 0, 0, 0)],
            init: vec![(0, 7)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0xFFFF_FFF9, r(x, 1)),
            rule: "Rd = imm − Rn",
        },
        Case {
            name: "ADC with carry",
            asm: "adc r1, r0, #1",
            code: vec![dp_imm(ADC, false, 1, 0, 1, 0)],
            init: vec![(0, 1)],
            nzcv: 0b0010,
            arm9_only: false,
            check: |x| (3, r(x, 1)),
            rule: "Rn + op2 + C",
        },
        Case {
            name: "SBC without carry",
            asm: "sbc r1, r0, #1",
            code: vec![dp_imm(SBC, false, 1, 0, 1, 0)],
            init: vec![(0, 5)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (3, r(x, 1)),
            rule: "Rn − op2 − !C",
        },
        Case {
            name: "AND/ORR/BIC/MVN",
            asm: "and r1,r0,#F0; orr r2,r0,#F; bic r3,r0,#F; mvn r4,r0",
            code: vec![
                dp_imm(AND, false, 1, 0, 0xF0, 0),
                dp_imm(ORR, false, 2, 0, 0xF, 0),
                dp_imm(BIC, false, 3, 0, 0xF, 0),
                dp_reg(MVN, false, 4, 0, 0, LSL, 0),
            ],
            init: vec![(0, 0x5A)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0x505F_50A5, r(x, 1) << 24 | r(x, 2) << 16 | r(x, 3) << 8 | r(x, 4) & 0xFF),
            rule: "5Ah: AND F0h=50h, ORR Fh=5Fh, BIC Fh=50h, MVN → …A5h",
        },
        Case {
            name: "LSL #0 keeps C",
            asm: "movs r1, r0, lsl #0",
            code: vec![dp_reg(MOV, true, 1, 0, 0, LSL, 0)],
            init: vec![(0, 1)],
            nzcv: 0b0010,
            arm9_only: false,
            check: |x| (1_2, r(x, 1) * 10 + x.nzcv() as u64),
            rule: "LSL #0: value unchanged, carry unchanged",
        },
        Case {
            name: "LSL by reg 32",
            asm: "movs r1, r0, lsl r2",
            code: vec![dp_rsr(MOV, true, 1, 0, 0, LSL, 2)],
            init: vec![(0, 1), (2, 32)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0x0_6, r(x, 1) << 4 | x.nzcv() as u64),
            rule: "LSL 32: result 0, C = bit 0",
        },
        Case {
            name: "LSR #32",
            asm: "movs r1, r0, lsr #32",
            code: vec![dp_reg(MOV, true, 1, 0, 0, LSR, 0)],
            init: vec![(0, 0x8000_0000)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0x0_6, r(x, 1) << 4 | x.nzcv() as u64),
            rule: "encoded LSR #0 means LSR #32: 0, C = bit 31",
        },
        Case {
            name: "ASR #32",
            asm: "movs r1, r0, asr #32",
            code: vec![dp_reg(MOV, true, 1, 0, 0, ASR, 0)],
            init: vec![(0, 0x8000_0000)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0xFFFF_FFFF_A, r(x, 1) << 4 | x.nzcv() as u64),
            rule: "ASR #32: all sign bits, C = bit 31",
        },
        Case {
            name: "ROR #4",
            asm: "mov r1, r0, ror #4",
            code: vec![dp_reg(MOV, false, 1, 0, 0, ROR, 4)],
            init: vec![(0, 0x1234_5678)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0x8123_4567, r(x, 1)),
            rule: "rotate right",
        },
        Case {
            name: "RRX",
            asm: "movs r1, r0, rrx",
            code: vec![dp_reg(MOV, true, 1, 0, 0, ROR, 0)],
            init: vec![(0, 2)],
            nzcv: 0b0010,
            arm9_only: false,
            check: |x| (0x8000_0001_8, r(x, 1) << 4 | x.nzcv() as u64),
            rule: "ROR #0 = RRX: C in at bit 31, bit 0 out to C",
        },
        Case {
            name: "CMP + conditional",
            asm: "cmp r0,#1; moveq r1,#1; movne r2,#1",
            code: vec![
                dp_imm(CMP, true, 0, 0, 1, 0),
                cond(dp_imm(MOV, false, 1, 0, 1, 0), 0),
                cond(dp_imm(MOV, false, 2, 0, 1, 0), 1),
            ],
            init: vec![(0, 1)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0x10, r(x, 1) << 4 | r(x, 2)),
            rule: "EQ executes, NE skipped",
        },
        Case {
            name: "MUL",
            asm: "mul r1, r0, r2",
            code: vec![mul(1, 0, 2)],
            init: vec![(0, 7), (2, 6)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (42, r(x, 1)),
            rule: "Rd = Rm·Rs",
        },
        Case {
            name: "MLA",
            asm: "mla r1, r0, r2, r3",
            code: vec![mla(1, 0, 2, 3)],
            init: vec![(0, 7), (2, 6), (3, 100)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (142, r(x, 1)),
            rule: "Rd = Rm·Rs + Rn",
        },
        Case {
            name: "UMULL",
            asm: "umull r1, r2, r0, r0",
            code: vec![umull(1, 2, 0, 0)],
            init: vec![(0, 0xFFFF_FFFF)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0xFFFF_FFFE_0000_0001, r(x, 2) << 32 | r(x, 1)),
            rule: "64-bit unsigned product",
        },
        Case {
            name: "SMULL",
            asm: "smull r1, r2, r0, r3",
            code: vec![smull(1, 2, 0, 3)],
            init: vec![(0, (-2i32) as u32), (3, 3)],
            nzcv: 0,
            arm9_only: false,
            check: |x| ((-6i64) as u64, r(x, 2) << 32 | r(x, 1)),
            rule: "64-bit signed product",
        },
        Case {
            name: "STR/LDR",
            asm: "str r1,[r0]; ldr r2,[r0]",
            code: vec![str_(1, 0, 0), ldr(2, 0, 0)],
            init: vec![(0, base), (1, 0xDEAD_BEEF)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0xDEAD_BEEF, r(x, 2)),
            rule: "word store/load",
        },
        Case {
            name: "LDR unaligned",
            asm: "str r1,[r0]; ldr r2,[r0,#1]",
            code: vec![str_(1, 0, 0), ldr(2, 0, 1)],
            init: vec![(0, base), (1, 0x1122_3344)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0x4411_2233, r(x, 2)),
            rule: "misaligned LDR rotates the aligned word by 8·(addr&3)",
        },
        Case {
            name: "LDRB",
            asm: "str r1,[r0]; ldrb r2,[r0,#2]",
            code: vec![str_(1, 0, 0), ldrb(2, 0, 2)],
            init: vec![(0, base), (1, 0x1122_3344)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0x22, r(x, 2)),
            rule: "zero-extended byte",
        },
        Case {
            name: "LDRSB/LDRSH/LDRH",
            asm: "str r1,[r0]; ldrsb r2,[r0]; ldrsh r3,[r0,#2]; ldrh r4,[r0,#2]",
            code: vec![
                str_(1, 0, 0),
                half(0x0010_00D0, 2, 0, 0),
                half(0x0010_00F0, 3, 0, 2),
                half(0x0010_00B0, 4, 0, 2),
            ],
            init: vec![(0, base), (1, 0x8001_0080)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0xFFFF_FF80_FFFF_8001, (r(x, 2) & 0xFFFF_FFFF) << 32 | r(x, 3)),
            rule: "sign-extended byte / halfword",
        },
        Case {
            name: "STMIA/LDMIA !",
            asm: "stmia r0!,{r1-r3}; ldmia r4!,{r5-r7}",
            code: vec![stmia_w(0, 0b1110), ldmia_w(4, 0b1110_0000)],
            init: vec![(0, base), (1, 11), (2, 22), (3, 33), (4, base)],
            nzcv: 0,
            arm9_only: false,
            check: |x| {
                (
                    ((CODE + 0x100 + 12) as u64) << 32 | 11 << 16 | 22 << 8 | 33,
                    r(x, 0) << 32 | r(x, 5) << 16 | r(x, 6) << 8 | r(x, 7),
                )
            },
            rule: "ascending, write-back = base + 4·count",
        },
        Case {
            name: "B skips",
            asm: "b +0 (skip next); mov r1,#1; mov r2,#2",
            code: vec![b(0), dp_imm(MOV, false, 1, 0, 1, 0), dp_imm(MOV, false, 2, 0, 2, 0)],
            init: vec![],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0x02, r(x, 1) << 4 | r(x, 2)),
            rule: "target = PC(+8) + 4·offset",
        },
        Case {
            name: "BL sets LR",
            asm: "bl +0; mov r1,#1",
            code: vec![bl(0), dp_imm(MOV, false, 1, 0, 1, 0)],
            init: vec![],
            nzcv: 0,
            arm9_only: false,
            check: |x| ((CODE + 4) as u64, r(x, 14)),
            rule: "LR = address of the next instruction",
        },
        Case {
            name: "CLZ (v5)",
            asm: "clz r1, r0",
            code: vec![AL | 0x016F_0F10 | 1 << 12],
            init: vec![(0, 0x0001_0000)],
            nzcv: 0,
            arm9_only: true,
            check: |x| (15, r(x, 1)),
            rule: "count leading zeros (ARM9 only)",
        },
        Case {
            name: "QADD saturates",
            asm: "qadd r1, r0, r2",
            code: vec![AL | 0x0100_0050 | 2 << 16 | 1 << 12],
            init: vec![(0, 0x7FFF_FFFF), (2, 1)],
            nzcv: 0,
            arm9_only: true,
            check: |x| (0x7FFF_FFFF_1, r(x, 1) << 4 | (x.cpsr() >> 27 & 1) as u64),
            rule: "saturate to 7FFFFFFFh and set Q (ARM9 only)",
        },
        Case {
            name: "SMULBB",
            asm: "smulbb r1, r0, r2",
            code: vec![AL | 0x0160_0080 | 1 << 16 | 2 << 8],
            init: vec![(0, 0x1234_FFFF), (2, 2)],
            nzcv: 0,
            arm9_only: true,
            check: |x| ((-2i32) as u32 as u64, r(x, 1)),
            rule: "signed 16×16 of the bottom halves (ARM9 only)",
        },
        Case {
            name: "MRS CPSR",
            asm: "mrs r1, cpsr",
            code: vec![AL | 0x010F_0000 | 1 << 12],
            init: vec![],
            nzcv: 0b0100,
            arm9_only: false,
            check: |x| (0x4000_00D3, r(x, 1)),
            rule: "reads Z flag + SVC mode, I, F (direct-boot state D3h)",
        },
        Case {
            name: "SWP",
            asm: "str r1,[r0]; swp r2, r3, [r0]; ldr r4,[r0]",
            code: vec![str_(1, 0, 0), AL | 0x0100_0090 | 2 << 12 | 3, ldr(4, 0, 0)],
            init: vec![(0, base), (1, 0xAAAA), (3, 0xBBBB)],
            nzcv: 0,
            arm9_only: false,
            check: |x| (0xAAAA_BBBB, r(x, 2) << 16 | r(x, 4)),
            rule: "Rd = [Rn]; [Rn] = Rm atomically",
        },
        Case {
            name: "BX to THUMB",
            asm: "add r0, pc, #1; bx r0; (thumb) mov r1, #7",
            code: vec![dp_imm(ADD, false, 0, 15, 1, 0), AL | 0x012F_FF10, 0xE7FE_2107],
            init: vec![],
            nzcv: 0,
            arm9_only: false,
            check: |x| (7 << 1 | 1, r(x, 1) << 1 | x.cpu.regs.get_t() as u64),
            rule: "BX with bit 0 set switches to THUMB",
        },
    ]
}

#[test]
fn every_case_on_arm9() {
    for (i, c) in cases().into_iter().enumerate() {
        let run = run::<true>(&format!("arm9_case{i}"), &c.code, false, &c.init, c.nzcv);
        let (want, got) = (c.check)(&run);
        assert_eq!(got, want, "{}: {}", c.name, c.asm);
    }
}

#[test]
fn report() {
    let mut doc = Doc::new("arm/arm.md", "ARM (32-bit) instruction set", "arm/arm.rs");
    doc.source(
        "hand-encoded programs executed on `ARM<true>` (ARM946E-S) and `ARM<false>` (ARM7TDMI)",
    );
    doc.p("Dispatch: `arm_lut[((instr >> 16) & 0xFF0) | ((instr >> 4) & 0xF)]` — bits 27-20 and 7-4 select one of 4096 handlers, built once per core by `arm_lut()`.");
    doc.bitfield(
        "Data processing",
        32,
        &[
            (31, 28, "COND"),
            (27, 26, "00"),
            (25, 25, "I"),
            (24, 21, "OPCODE"),
            (20, 20, "S"),
            (19, 16, "Rn"),
            (15, 12, "Rd"),
            (11, 0, "OPERAND 2"),
        ],
    );
    doc.bitfield("  operand 2 (I=1)", 12, &[(11, 8, "ROT"), (7, 0, "IMM8")]);
    doc.bitfield(
        "  operand 2 (I=0)",
        12,
        &[(11, 7, "SHIFT IMM / Rs,0"), (6, 5, "TYPE"), (4, 4, "R"), (3, 0, "Rm")],
    );
    doc.bitfield(
        "Single data transfer",
        32,
        &[
            (31, 28, "COND"),
            (27, 26, "01"),
            (25, 25, "I"),
            (24, 24, "P"),
            (23, 23, "U"),
            (22, 22, "B"),
            (21, 21, "W"),
            (20, 20, "L"),
            (19, 16, "Rn"),
            (15, 12, "Rd"),
            (11, 0, "OFFSET"),
        ],
    );
    doc.bitfield(
        "Block transfer",
        32,
        &[
            (31, 28, "COND"),
            (27, 25, "100"),
            (24, 24, "P"),
            (23, 23, "U"),
            (22, 22, "S"),
            (21, 21, "W"),
            (20, 20, "L"),
            (19, 16, "Rn"),
            (15, 0, "REGISTER LIST"),
        ],
    );
    doc.bitfield(
        "Branch",
        32,
        &[(31, 28, "COND"), (27, 25, "101"), (24, 24, "L"), (23, 0, "OFFSET (words, signed)")],
    );
    doc.bitfield(
        "Multiply",
        32,
        &[
            (31, 28, "COND"),
            (27, 22, "000000"),
            (21, 21, "A"),
            (20, 20, "S"),
            (19, 16, "Rd"),
            (15, 12, "Rn"),
            (11, 8, "Rs"),
            (7, 4, "1001"),
            (3, 0, "Rm"),
        ],
    );
    doc.table(
        &[("Opcode", L), ("Mnemonic", L), ("Operation", L)],
        &[
            vec!["0", "AND", "Rd = Rn & op2"],
            vec!["1", "EOR", "Rd = Rn ^ op2"],
            vec!["2", "SUB", "Rd = Rn − op2"],
            vec!["3", "RSB", "Rd = op2 − Rn"],
            vec!["4", "ADD", "Rd = Rn + op2"],
            vec!["5", "ADC", "Rd = Rn + op2 + C"],
            vec!["6", "SBC", "Rd = Rn − op2 − !C"],
            vec!["7", "RSC", "Rd = op2 − Rn − !C"],
            vec!["8", "TST", "flags of Rn & op2"],
            vec!["9", "TEQ", "flags of Rn ^ op2"],
            vec!["A", "CMP", "flags of Rn − op2"],
            vec!["B", "CMN", "flags of Rn + op2"],
            vec!["C", "ORR", "Rd = Rn | op2"],
            vec!["D", "MOV", "Rd = op2"],
            vec!["E", "BIC", "Rd = Rn & !op2"],
            vec!["F", "MVN", "Rd = !op2"],
        ],
    );
    let mut ck = Checks::new();
    let mut rows = Vec::new();
    for (i, c) in cases().into_iter().enumerate() {
        let a9 = run::<true>(&format!("arm9_report{i}"), &c.code, false, &c.init, c.nzcv);
        let (want, got9) = (c.check)(&a9);
        ck.eq(&format!("ARM9 {}", c.name), c.rule, format!("{want:X}"), format!("{got9:X}"));
        let a7 = if c.arm9_only {
            "n/a (ARMv5 only)".to_string()
        } else {
            let r7 = run::<false>(&format!("arm7_report{i}"), &c.code, false, &c.init, c.nzcv);
            // Reuse the ARM9 checker on the ARM7 register file.
            let shim = Run::<true> {
                cpu: {
                    let mut cpu = a9.cpu;
                    cpu.regs = r7.cpu.regs.clone();
                    cpu
                },
                nds: r7.nds,
            };
            let (_, got7) = (c.check)(&shim);
            ck.eq(&format!("ARM7 {}", c.name), c.rule, format!("{want:X}"), format!("{got7:X}"));
            if got7 == want { "✅".into() } else { "❌".into() }
        };
        let enc: Vec<String> = c.code.iter().map(|w| format!("`{w:08X}`")).collect();
        rows.push(vec![
            c.name.into(),
            format!("`{}`", c.asm),
            enc.join(" "),
            c.rule.into(),
            if got9 == want { "✅" } else { "❌" }.into(),
            a7,
        ]);
    }
    doc.h2("Executed cases");
    doc.p("`Expected`/`Lunaris` in the checks below pack the inspected registers (and NZCV in the low nibble where flags matter) into one hex number.");
    doc.table(
        &[("Case", L), ("Assembly", L), ("Encoding", L), ("Rule", L), ("ARM9", L), ("ARM7", L)],
        &rows,
    );
    doc.h2("Spec checks");
    ck.write(&mut doc);
    doc.save();
    ck.finish();
}
