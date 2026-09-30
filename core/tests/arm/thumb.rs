//! Spec tests for `core/src/arm/thumb.rs` — the 16-bit THUMB instruction
//! set (all 19 GBATEK formats that the cases touch), on both cores.
//!
//! GBATEK "THUMB Instruction Set": <https://problemkaputt.de/gbatek.htm#thumbinstructionsummary>

use crate::{
    arm::spec::{CODE, Run, run},
    test_support::{
        md::{Doc, L, R},
        spec::Checks,
    },
};

type Check = fn(&Run<true>) -> (u64, u64);

struct Case {
    fmt: u32,
    name: &'static str,
    asm: &'static str,
    code: Vec<u32>,
    init: Vec<(u32, u32)>,
    nzcv: u32,
    check: Check,
}

fn r(x: &Run<true>, i: u32) -> u64 {
    x.r(i) as u64
}

fn alu(op: u32, rd: u32, rs: u32) -> u32 {
    0x4000 | op << 6 | rs << 3 | rd
}

fn cases() -> Vec<Case> {
    let base = CODE + 0x100;
    vec![
        Case {
            fmt: 1,
            name: "LSL imm",
            asm: "lsls r1, r0, #4",
            code: vec![4 << 6 | 1],
            init: vec![(0, 0x1000_0001)],
            nzcv: 0,
            check: |x| (0x10_2, r(x, 1) << 4 | x.nzcv() as u64),
        },
        Case {
            fmt: 1,
            name: "LSR imm",
            asm: "lsrs r1, r0, #1",
            code: vec![0x0800 | 1 << 6 | 1],
            init: vec![(0, 3)],
            nzcv: 0,
            check: |x| (0x1_2, r(x, 1) << 4 | x.nzcv() as u64),
        },
        Case {
            fmt: 1,
            name: "ASR imm",
            asm: "asrs r1, r0, #4",
            code: vec![0x1000 | 4 << 6 | 1],
            init: vec![(0, 0x8000_0000)],
            nzcv: 0,
            check: |x| (0xF800_0000_8, r(x, 1) << 4 | x.nzcv() as u64),
        },
        Case {
            fmt: 2,
            name: "ADD reg",
            asm: "adds r2, r0, r1",
            code: vec![0x1800 | 1 << 6 | 2],
            init: vec![(0, 0xFFFF_FFFF), (1, 1)],
            nzcv: 0,
            check: |x| (0x0_6, r(x, 2) << 4 | x.nzcv() as u64),
        },
        Case {
            fmt: 2,
            name: "SUB imm3",
            asm: "subs r2, r0, #3",
            code: vec![0x1E00 | 3 << 6 | 2],
            init: vec![(0, 10)],
            nzcv: 0,
            check: |x| (7, r(x, 2)),
        },
        Case {
            fmt: 3,
            name: "MOV imm8",
            asm: "movs r3, #200",
            code: vec![0x2000 | 3 << 8 | 200],
            init: vec![],
            nzcv: 0,
            check: |x| (200, r(x, 3)),
        },
        Case {
            fmt: 3,
            name: "CMP imm8",
            asm: "cmp r3, #5",
            code: vec![0x2800 | 3 << 8 | 5],
            init: vec![(3, 5)],
            nzcv: 0,
            check: |x| (0x6, x.nzcv() as u64),
        },
        Case {
            fmt: 3,
            name: "ADD/SUB imm8",
            asm: "adds r3,#10; subs r3,#3",
            code: vec![0x3000 | 3 << 8 | 10, 0x3800 | 3 << 8 | 3],
            init: vec![(3, 1)],
            nzcv: 0,
            check: |x| (8, r(x, 3)),
        },
        Case {
            fmt: 4,
            name: "ALU AND/EOR/ORR/BIC",
            asm: "ands r1,r0; eors r2,r0; orrs r3,r0; bics r4,r0",
            code: vec![alu(0, 1, 0), alu(1, 2, 0), alu(12, 3, 0), alu(14, 4, 0)],
            init: vec![(0, 0x0F), (1, 0x3C), (2, 0x3C), (3, 0x30), (4, 0xFF)],
            nzcv: 0,
            check: |x| (0x0C_33_3F_F0, r(x, 1) << 24 | r(x, 2) << 16 | r(x, 3) << 8 | r(x, 4)),
        },
        Case {
            fmt: 4,
            name: "ALU NEG/MVN",
            asm: "negs r1, r0; mvns r2, r0",
            code: vec![alu(9, 1, 0), alu(15, 2, 0)],
            init: vec![(0, 5)],
            nzcv: 0,
            check: |x| (0xFFFF_FFFB_FFFF_FFFA, r(x, 1) << 32 | r(x, 2)),
        },
        Case {
            fmt: 4,
            name: "ALU MUL",
            asm: "muls r1, r0",
            code: vec![alu(13, 1, 0)],
            init: vec![(0, 9), (1, 7)],
            nzcv: 0,
            check: |x| (63, r(x, 1)),
        },
        Case {
            fmt: 4,
            name: "ALU ADC/SBC",
            asm: "adcs r1, r0; sbcs r2, r0",
            code: vec![alu(5, 1, 0), alu(6, 2, 0)],
            init: vec![(0, 1), (1, 1), (2, 10)],
            nzcv: 0b0010,
            check: |x| (3 << 8 | 8, r(x, 1) << 8 | r(x, 2)),
        },
        Case {
            fmt: 4,
            name: "ALU ROR reg",
            asm: "rors r1, r0",
            code: vec![alu(7, 1, 0)],
            init: vec![(0, 8), (1, 0x0000_00FF)],
            nzcv: 0,
            check: |x| (0xFF00_0000, r(x, 1)),
        },
        Case {
            fmt: 4,
            name: "ALU TST/CMN",
            asm: "tst r1, r0",
            code: vec![alu(8, 1, 0)],
            init: vec![(0, 0xF0), (1, 0x0F)],
            nzcv: 0,
            check: |x| (0x4, x.nzcv() as u64),
        },
        Case {
            fmt: 5,
            name: "hi-reg MOV",
            asm: "mov r8, r0; mov r1, r8",
            code: vec![0x4600 | 1 << 7 | 0, 0x4600 | 1 << 6 | 0 << 3 | 1],
            init: vec![(0, 0x1234)],
            nzcv: 0,
            check: |x| (0x1234_1234, r(x, 8) << 16 | r(x, 1)),
        },
        Case {
            fmt: 5,
            name: "hi-reg ADD",
            asm: "add r9, r0",
            code: vec![0x4400 | 1 << 7 | 1],
            init: vec![(0, 5), (9, 10)],
            nzcv: 0,
            check: |x| (15, r(x, 9)),
        },
        Case {
            fmt: 6,
            name: "LDR PC-relative",
            asm: "ldr r0, [pc, #4]",
            code: vec![0x4801, 0x46C0, 0x46C0, 0xE7FE, 0x5678, 0x1234],
            init: vec![],
            nzcv: 0,
            check: |x| (0x1234_5678, r(x, 0)),
        },
        Case {
            fmt: 7,
            name: "STR/LDR reg offset",
            asm: "str r1,[r0,r2]; ldr r3,[r0,r2]",
            code: vec![0x5000 | 2 << 6 | 0 << 3 | 1, 0x5800 | 2 << 6 | 0 << 3 | 3],
            init: vec![(0, base), (1, 0xCAFE_BABE), (2, 8)],
            nzcv: 0,
            check: |x| (0xCAFE_BABE, r(x, 3)),
        },
        Case {
            fmt: 8,
            name: "LDRSB/LDRSH",
            asm: "strh r1,[r0,r2]; ldsb r3,[r0,r2]; ldsh r4,[r0,r2]",
            code: vec![0x5200 | 2 << 6 | 1, 0x5600 | 2 << 6 | 3, 0x5E00 | 2 << 6 | 4],
            init: vec![(0, base), (1, 0x80F0), (2, 0)],
            nzcv: 0,
            check: |x| (0xFFFF_FFF0_FFFF_80F0, r(x, 3) << 32 | r(x, 4)),
        },
        Case {
            fmt: 9,
            name: "STR/LDR imm5",
            asm: "str r1,[r0,#4]; ldr r2,[r0,#4]",
            code: vec![0x6000 | 1 << 6 | 1, 0x6800 | 1 << 6 | 2],
            init: vec![(0, base), (1, 77)],
            nzcv: 0,
            check: |x| (77, r(x, 2)),
        },
        Case {
            fmt: 9,
            name: "STRB/LDRB imm5",
            asm: "strb r1,[r0,#3]; ldrb r2,[r0,#3]",
            code: vec![0x7000 | 3 << 6 | 1, 0x7800 | 3 << 6 | 2],
            init: vec![(0, base), (1, 0x1FF)],
            nzcv: 0,
            check: |x| (0xFF, r(x, 2)),
        },
        Case {
            fmt: 10,
            name: "STRH/LDRH imm5",
            asm: "strh r1,[r0,#2]; ldrh r2,[r0,#2]",
            code: vec![0x8000 | 1 << 6 | 1, 0x8800 | 1 << 6 | 2],
            init: vec![(0, base), (1, 0x12345)],
            nzcv: 0,
            check: |x| (0x2345, r(x, 2)),
        },
        Case {
            fmt: 11,
            name: "SP-relative",
            asm: "str r1,[sp,#8]; ldr r2,[sp,#8]",
            code: vec![0x9000 | 1 << 8 | 2, 0x9800 | 2 << 8 | 2],
            init: vec![(13, base), (1, 99)],
            nzcv: 0,
            check: |x| (99, r(x, 2)),
        },
        Case {
            fmt: 12,
            name: "ADD rd, PC/SP",
            asm: "add r0, pc, #8; add r1, sp, #8",
            code: vec![0xA000 | 2, 0xA800 | 1 << 8 | 2],
            init: vec![(13, 0x1000)],
            nzcv: 0,
            check: |x| (((CODE + 4 + 8) as u64) << 32 | 0x1008, r(x, 0) << 32 | r(x, 1)),
        },
        Case {
            fmt: 13,
            name: "ADD SP, #±",
            asm: "add sp, #16; sub sp, #4",
            code: vec![0xB000 | 4, 0xB080 | 1],
            init: vec![(13, 0x1000)],
            nzcv: 0,
            check: |x| (0x100C, r(x, 13)),
        },
        Case {
            fmt: 14,
            name: "PUSH/POP",
            asm: "push {r0,r1,lr}; pop {r2,r3,r4}",
            code: vec![0xB400 | 0x100 | 0b11, 0xBC00 | 0b1_1100],
            init: vec![(13, base + 0x40), (0, 1), (1, 2), (14, 3)],
            nzcv: 0,
            check: |x| {
                (
                    0x123_0000 | (CODE + 0x100 + 0x40) as u64 & 0xFFFF,
                    r(x, 2) << 24 | r(x, 3) << 20 | r(x, 4) << 16 | r(x, 13) & 0xFFFF,
                )
            },
        },
        Case {
            fmt: 15,
            name: "STMIA/LDMIA",
            asm: "stmia r0!,{r1,r2}; ldmia r3!,{r4,r5}",
            code: vec![0xC000 | 0 << 8 | 0b110, 0xC800 | 3 << 8 | 0b11_0000],
            init: vec![(0, base), (1, 5), (2, 6), (3, base)],
            nzcv: 0,
            check: |x| {
                (
                    ((CODE + 0x100 + 8) as u64) << 16 | 5 << 8 | 6,
                    r(x, 0) << 16 | r(x, 4) << 8 | r(x, 5),
                )
            },
        },
        Case {
            fmt: 16,
            name: "Bcond taken",
            asm: "cmp r0,#0; beq +2; movs r1,#1; movs r2,#2",
            code: vec![0x2800, 0xD000, 0x2101, 0x2202],
            init: vec![(0, 0)],
            nzcv: 0,
            check: |x| (0x02, r(x, 1) << 4 | r(x, 2)),
        },
        Case {
            fmt: 16,
            name: "Bcond not taken",
            asm: "cmp r0,#0; bne +2; movs r1,#1",
            code: vec![0x2800, 0xD100, 0x2101],
            init: vec![(0, 0)],
            nzcv: 0,
            check: |x| (1, r(x, 1)),
        },
        Case {
            fmt: 18,
            name: "B uncond",
            asm: "b +2; movs r1,#1; movs r2,#2",
            code: vec![0xE000, 0x2101, 0x2202],
            init: vec![],
            nzcv: 0,
            check: |x| (0x02, r(x, 1) << 4 | r(x, 2)),
        },
        Case {
            fmt: 19,
            name: "BL (two halves)",
            asm: "bl +4; movs r1,#1; movs r2,#2",
            code: vec![0xF000, 0xF800 | 1, 0x2101, 0x2202],
            init: vec![],
            nzcv: 0,
            check: |x| ((CODE + 4 | 1) as u64, r(x, 14)),
        },
        Case {
            fmt: 5,
            name: "BX to ARM",
            asm: "bx r0 → (ARM) mov r1, #9",
            code: vec![0x4700, 0x46C0, 0x1009, 0xE3A0, 0xFFFE, 0xEAFF],
            init: vec![(0, CODE + 4)],
            nzcv: 0,
            check: |x| (9 << 1, r(x, 1) << 1 | x.cpu.regs.get_t() as u64),
        },
    ]
}

#[test]
fn every_case_on_arm9() {
    for (i, c) in cases().into_iter().enumerate() {
        let run = run::<true>(&format!("thumb9_case{i}"), &c.code, true, &c.init, c.nzcv);
        let (want, got) = (c.check)(&run);
        assert_eq!(got, want, "format {} {}: {}", c.fmt, c.name, c.asm);
    }
}

#[test]
fn report() {
    let mut doc = Doc::new("arm/thumb.md", "THUMB (16-bit) instruction set", "arm/thumb.rs");
    doc.source("hand-encoded programs executed in THUMB state on both cores");
    doc.p("Dispatch: `thumb_lut[instr >> 8]` (256 handlers). GBATEK groups THUMB into 19 formats; the cases below exercise each one that has a handler:");
    doc.table(
        &[("Fmt", R), ("Bits 15-8 pattern", L), ("Meaning", L)],
        &[
            vec!["1", "000o oiii", "LSL/LSR/ASR Rd, Rs, #imm5"],
            vec!["2", "0001 1Ioo", "ADD/SUB Rd, Rs, Rn/#imm3"],
            vec!["3", "001o oddd", "MOV/CMP/ADD/SUB Rd, #imm8"],
            vec![
                "4",
                "0100 00oo",
                "ALU: AND EOR LSL LSR ASR ADC SBC ROR TST NEG CMP CMN ORR MUL BIC MVN",
            ],
            vec!["5", "0100 01oo", "hi-register ADD/CMP/MOV, BX/BLX"],
            vec!["6", "0100 1ddd", "LDR Rd, [PC, #imm8·4] (PC word-aligned)"],
            vec!["7", "0101 LB0o", "STR/STRB/LDR/LDRB [Rb, Ro]"],
            vec!["8", "0101 HS1o", "STRH/LDSB/LDRH/LDSH [Rb, Ro]"],
            vec!["9", "011B Liii", "STR/LDR(B) [Rb, #imm5]"],
            vec!["10", "1000 Liii", "STRH/LDRH [Rb, #imm5·2]"],
            vec!["11", "1001 Lddd", "STR/LDR [SP, #imm8·4]"],
            vec!["12", "1010 Sddd", "ADD Rd, PC/SP, #imm8·4"],
            vec!["13", "1011 0000", "ADD SP, #±imm7·4"],
            vec!["14", "1011 L10R", "PUSH/POP {rlist}(+LR/PC)"],
            vec!["15", "1100 Lbbb", "STMIA/LDMIA Rb!, {rlist}"],
            vec!["16", "1101 cccc", "B{cond} ±imm8·2"],
            vec!["17", "1101 1111", "SWI"],
            vec!["18", "1110 0iii", "B ±imm11·2"],
            vec!["19", "1111 Hiii", "BL (two halves: high then low offset)"],
        ],
    );
    let mut ck = Checks::new();
    let mut rows = Vec::new();
    for (i, c) in cases().into_iter().enumerate() {
        let a9 = run::<true>(&format!("thumb9_report{i}"), &c.code, true, &c.init, c.nzcv);
        let (want, got9) = (c.check)(&a9);
        ck.eq(
            &format!("ARM9 fmt{} {}", c.fmt, c.name),
            c.asm,
            format!("{want:X}"),
            format!("{got9:X}"),
        );
        let r7 = run::<false>(&format!("thumb7_report{i}"), &c.code, true, &c.init, c.nzcv);
        let shim = Run::<true> {
            cpu: {
                let mut cpu = a9.cpu;
                cpu.regs = r7.cpu.regs.clone();
                cpu
            },
            nds: r7.nds,
        };
        let (_, got7) = (c.check)(&shim);
        ck.eq(
            &format!("ARM7 fmt{} {}", c.fmt, c.name),
            c.asm,
            format!("{want:X}"),
            format!("{got7:X}"),
        );
        let enc: Vec<String> = c.code.iter().map(|h| format!("`{h:04X}`")).collect();
        rows.push(vec![
            c.fmt.to_string(),
            c.name.into(),
            format!("`{}`", c.asm),
            enc.join(" "),
            if got9 == want { "✅" } else { "❌" }.into(),
            if got7 == want { "✅" } else { "❌" }.into(),
        ]);
    }
    doc.h2("Executed cases");
    doc.table(
        &[("Fmt", R), ("Case", L), ("Assembly", L), ("Encoding", L), ("ARM9", L), ("ARM7", L)],
        &rows,
    );
    doc.h2("Spec checks");
    ck.write(&mut doc);
    doc.save();
    ck.finish();
}
