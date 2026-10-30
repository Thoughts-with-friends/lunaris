//! The bundled Verilog samples and a small RV32I encoder to write programs
//! for the RISC-V sample.
//!
//! ```text
//!  toggle_switch()  hdl/toggle_switch/toggle_switch.v  push-on/push-off switch + LED
//!  riscv_rv32i()    hdl/riscv_rv32i/riscv_rv32i.v    RV32I CPU, IMEM/DMEM, mailbox
//!  fibonacci(n)     program: DMEM[i] = fib(i), MAILBOX_OUT = fib(n), ECALL
//! ```

use crate::spec::BlockSpec;

pub const TOGGLE_SWITCH_V: &str = include_str!("../hdl/toggle_switch/toggle_switch.v");
pub const RISCV_RV32I_V: &str = include_str!("../hdl/riscv_rv32i/riscv_rv32i.v");

/// Offsets inside the RISC-V block (see `hdl/riscv_rv32i/riscv_rv32i.v`).
pub mod riscv_map {
    pub const CTRL: u32 = 0x0000;
    pub const STATUS: u32 = 0x0004;
    pub const PC: u32 = 0x0008;
    pub const INSTRET: u32 = 0x000C;
    pub const MAILBOX_OUT: u32 = 0x0010;
    pub const MAILBOX_IN: u32 = 0x0014;
    pub const IRQ_EN: u32 = 0x0018;
    pub const IRQ_STATUS: u32 = 0x001C;
    pub const IMEM: u32 = 0x1000;
    pub const DMEM: u32 = 0x2000;
    /// CTRL value: reset the core and run.
    pub const RESET_AND_RUN: u32 = 0b11;
    /// STATUS bit: halted (ECALL / EBREAK / illegal).
    pub const HALTED: u32 = 0b10;
}

/// # Panics
/// Never: the bundled source is checked by the tests.
#[expect(clippy::expect_used, reason = "bundled, test-checked source")]
pub fn toggle_switch() -> BlockSpec {
    BlockSpec::from_verilog(TOGGLE_SWITCH_V).expect("bundled sample is valid")
}

/// # Panics
/// Never: the bundled source is checked by the tests.
#[expect(clippy::expect_used, reason = "bundled, test-checked source")]
pub fn riscv_rv32i() -> BlockSpec {
    BlockSpec::from_verilog(RISCV_RV32I_V).expect("bundled sample is valid")
}

/// RV32I instruction encoders (`rd, rs1, rs2` are register numbers).
pub mod rv32 {
    const fn r(f7: u32, rs2: u32, rs1: u32, f3: u32, rd: u32, op: u32) -> u32 {
        (f7 << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
    }
    const fn i(imm: i32, rs1: u32, f3: u32, rd: u32, op: u32) -> u32 {
        (((imm as u32) & 0xFFF) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
    }
    const fn s(imm: i32, rs2: u32, rs1: u32, f3: u32) -> u32 {
        let imm = imm as u32;
        (((imm >> 5) & 0x7F) << 25)
            | (rs2 << 20)
            | (rs1 << 15)
            | (f3 << 12)
            | ((imm & 0x1F) << 7)
            | 0x23
    }
    const fn b(imm: i32, rs1: u32, rs2: u32, f3: u32) -> u32 {
        let imm = imm as u32;
        (((imm >> 12) & 1) << 31)
            | (((imm >> 5) & 0x3F) << 25)
            | (rs2 << 20)
            | (rs1 << 15)
            | (f3 << 12)
            | (((imm >> 1) & 0xF) << 8)
            | (((imm >> 11) & 1) << 7)
            | 0x63
    }

    pub const fn lui(rd: u32, upper: u32) -> u32 {
        (upper << 12) | (rd << 7) | 0x37
    }
    pub const fn auipc(rd: u32, upper: u32) -> u32 {
        (upper << 12) | (rd << 7) | 0x17
    }
    pub const fn jal(rd: u32, offset: i32) -> u32 {
        let imm = offset as u32;
        (((imm >> 20) & 1) << 31)
            | (((imm >> 1) & 0x3FF) << 21)
            | (((imm >> 11) & 1) << 20)
            | (((imm >> 12) & 0xFF) << 12)
            | (rd << 7)
            | 0x6F
    }
    pub const fn jalr(rd: u32, rs1: u32, offset: i32) -> u32 {
        i(offset, rs1, 0, rd, 0x67)
    }
    pub const fn beq(rs1: u32, rs2: u32, offset: i32) -> u32 {
        b(offset, rs1, rs2, 0)
    }
    pub const fn bne(rs1: u32, rs2: u32, offset: i32) -> u32 {
        b(offset, rs1, rs2, 1)
    }
    pub const fn blt(rs1: u32, rs2: u32, offset: i32) -> u32 {
        b(offset, rs1, rs2, 4)
    }
    pub const fn bge(rs1: u32, rs2: u32, offset: i32) -> u32 {
        b(offset, rs1, rs2, 5)
    }
    pub const fn bltu(rs1: u32, rs2: u32, offset: i32) -> u32 {
        b(offset, rs1, rs2, 6)
    }
    pub const fn bgeu(rs1: u32, rs2: u32, offset: i32) -> u32 {
        b(offset, rs1, rs2, 7)
    }
    pub const fn lb(rd: u32, rs1: u32, offset: i32) -> u32 {
        i(offset, rs1, 0, rd, 0x03)
    }
    pub const fn lh(rd: u32, rs1: u32, offset: i32) -> u32 {
        i(offset, rs1, 1, rd, 0x03)
    }
    pub const fn lw(rd: u32, rs1: u32, offset: i32) -> u32 {
        i(offset, rs1, 2, rd, 0x03)
    }
    pub const fn lbu(rd: u32, rs1: u32, offset: i32) -> u32 {
        i(offset, rs1, 4, rd, 0x03)
    }
    pub const fn lhu(rd: u32, rs1: u32, offset: i32) -> u32 {
        i(offset, rs1, 5, rd, 0x03)
    }
    pub const fn sb(rs2: u32, rs1: u32, offset: i32) -> u32 {
        s(offset, rs2, rs1, 0)
    }
    pub const fn sh(rs2: u32, rs1: u32, offset: i32) -> u32 {
        s(offset, rs2, rs1, 1)
    }
    pub const fn sw(rs2: u32, rs1: u32, offset: i32) -> u32 {
        s(offset, rs2, rs1, 2)
    }
    pub const fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(imm, rs1, 0, rd, 0x13)
    }
    pub const fn slti(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(imm, rs1, 2, rd, 0x13)
    }
    pub const fn xori(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(imm, rs1, 4, rd, 0x13)
    }
    pub const fn srai(rd: u32, rs1: u32, shamt: i32) -> u32 {
        i(0x400 | shamt, rs1, 5, rd, 0x13)
    }
    pub const fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0, rs2, rs1, 0, rd, 0x33)
    }
    pub const fn sub(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x20, rs2, rs1, 0, rd, 0x33)
    }
    pub const fn sll(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0, rs2, rs1, 1, rd, 0x33)
    }
    pub const fn slt(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0, rs2, rs1, 2, rd, 0x33)
    }
    pub const fn sltu(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0, rs2, rs1, 3, rd, 0x33)
    }
    pub const fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0, rs2, rs1, 4, rd, 0x33)
    }
    pub const fn srl(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0, rs2, rs1, 5, rd, 0x33)
    }
    pub const fn sra(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x20, rs2, rs1, 5, rd, 0x33)
    }
    pub const fn or(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0, rs2, rs1, 6, rd, 0x33)
    }
    pub const fn and(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0, rs2, rs1, 7, rd, 0x33)
    }
    pub const ECALL: u32 = 0x0000_0073;
}

/// Fibonacci: `DMEM[i] = fib(i)` for `i < n`, then `MAILBOX_OUT = fib(n)`
/// (store to 0x8000_0000) and ECALL.
pub fn fibonacci(n: i32) -> Vec<u32> {
    use rv32::*;
    vec![
        addi(1, 0, 0), // a = 0
        addi(2, 0, 1), // b = 1
        addi(3, 0, n), // count
        addi(5, 0, 0), // DMEM pointer
        // loop:
        sw(1, 5, 0),   // DMEM[i] = a
        add(4, 1, 2),  // t = a + b
        addi(1, 2, 0), // a = b
        addi(2, 4, 0), // b = t
        addi(5, 5, 4),
        addi(3, 3, -1),
        bne(3, 0, -24), // -> loop
        lui(6, 0x80000),
        sw(1, 6, 0), // MAILBOX_OUT = a
        ECALL,
    ]
}

/// An endless counter: `MAILBOX_OUT` = 1, 2, 3, ... (never halts), for
/// watching the CPU live in the waveform window.
pub fn counter() -> Vec<u32> {
    use rv32::*;
    vec![
        lui(6, 0x80000), // MMIO base
        addi(1, 1, 1),   // loop: x1 += 1
        sw(1, 6, 0),     // MAILBOX_OUT = x1
        jal(0, -8),      // -> loop
    ]
}

#[cfg(test)]
mod tests {
    use ds_emu::slot2::{SLOT2_START, Slot2Device, Width};

    use super::{riscv_map as map, rv32::*, *};
    use crate::slot::FpgaSlot;

    const BASE: u32 = SLOT2_START;

    fn rd(slot: &mut FpgaSlot, offset: u32) -> u32 {
        slot.read(BASE + offset, Width::Word)
    }

    fn wr(slot: &mut FpgaSlot, offset: u32, value: u32) {
        slot.write(BASE + offset, value, Width::Word);
    }

    #[test]
    fn toggle_switch_toggles() {
        let mut slot = FpgaSlot::default();
        assert!(slot.add_spec(toggle_switch()).is_ok());
        assert_eq!(rd(&mut slot, 0), 0x5447_5357);
        wr(&mut slot, 0x10, 1); // IRQ_EN
        for press in 1..=3_u32 {
            wr(&mut slot, 0x04, 1);
            assert_eq!(rd(&mut slot, 0x08), press & 1, "LED after {press} presses");
            assert_eq!(rd(&mut slot, 0x0C), press);
            assert!(slot.tick(4), "a press raises the interrupt");
            wr(&mut slot, 0x14, 1); // acknowledge
            assert!(!slot.tick(4));
        }
        wr(&mut slot, 0x04, 0); // writing 0 is not a press
        assert_eq!(rd(&mut slot, 0x0C), 3);
    }

    fn run(program: &[u32]) -> FpgaSlot {
        let mut slot = FpgaSlot::default();
        assert!(slot.add_spec(riscv_rv32i()).is_ok());
        for (i, word) in program.iter().enumerate() {
            wr(&mut slot, map::IMEM + 4 * i as u32, *word);
        }
        wr(&mut slot, map::IRQ_EN, 0b11);
        wr(&mut slot, map::CTRL, map::RESET_AND_RUN);
        let div = riscv_rv32i().clock_div;
        for _ in 0..10_000 {
            slot.tick(div * 64);
            if rd(&mut slot, map::STATUS) & map::HALTED != 0 {
                return slot;
            }
        }
        panic!("program did not halt; PC = {:#X}", rd(&mut slot, map::PC));
    }

    #[test]
    fn riscv_computes_fibonacci() {
        let mut slot = run(&fibonacci(20));
        assert_eq!(rd(&mut slot, map::MAILBOX_OUT), 6765);
        let (mut a, mut b) = (0_u32, 1_u32);
        for i in 0..20 {
            assert_eq!(rd(&mut slot, map::DMEM + 4 * i), a, "fib({i})");
            (a, b) = (b, a + b);
        }
        assert_eq!(rd(&mut slot, map::STATUS) & 0b100, 0, "no illegal instruction");
        assert_eq!(rd(&mut slot, map::IRQ_STATUS), 0b11, "halt + mailbox interrupts");
        assert_eq!(rd(&mut slot, map::INSTRET), 4 + 7 * 20 + 2);
    }

    /// Every instruction class, results stored to DMEM and compared with
    /// what Rust computes.
    #[test]
    fn riscv_executes_rv32i() {
        let mut p = vec![
            addi(1, 0, -16),
            addi(2, 0, 3),
            sub(3, 1, 2),
            sw(3, 0, 0),
            slt(4, 1, 2),
            sw(4, 0, 4),
            sltu(5, 1, 2),
            sw(5, 0, 8),
            sra(6, 1, 2),
            sw(6, 0, 12),
            srl(7, 1, 2),
            sw(7, 0, 16),
            sll(8, 2, 2),
            sw(8, 0, 20),
            xor(9, 1, 2),
            sw(9, 0, 24),
            or(9, 1, 2),
            sw(9, 0, 28),
            and(9, 1, 2),
            sw(9, 0, 32),
            srai(10, 1, 1),
            sw(10, 0, 36),
            slti(11, 1, -15),
            sw(11, 0, 40),
            lui(12, 0x12345),
            addi(12, 12, 0x678),
            sw(12, 0, 44),
            sb(12, 0, 48),
            sh(12, 0, 50),
            sw(1, 0, 52),
            lb(13, 0, 52),
            sw(13, 0, 56),
            lbu(14, 0, 52),
            sw(14, 0, 60),
            lh(15, 0, 52),
            sw(15, 0, 64),
            lhu(16, 0, 52),
            sw(16, 0, 68),
        ];
        let jal_at = 4 * p.len() as u32;
        p.extend([jal(17, 8), addi(18, 0, 99), sw(17, 0, 72), sw(18, 0, 88)]);
        p.extend([
            addi(19, 0, 0),
            blt(1, 2, 8),
            addi(19, 19, 1),
            bltu(1, 2, 8),
            addi(19, 19, 2),
            bge(2, 1, 8),
            addi(19, 19, 4),
            bgeu(1, 2, 8),
            addi(19, 19, 8),
            beq(0, 0, 8),
            addi(19, 19, 16),
            sw(19, 0, 76),
        ]);
        let auipc_at = 4 * p.len() as u32;
        p.extend([auipc(20, 1), sw(20, 0, 80)]);
        let link_at = 4 * p.len() as u32;
        p.extend([
            auipc(22, 0),
            jalr(23, 22, 12),
            addi(23, 0, 0),
            sw(23, 0, 84),
            xori(24, 1, -1),
            sw(24, 0, 92),
            ECALL,
        ]);

        let mut slot = run(&p);
        let dmem = |slot: &mut FpgaSlot, offset: u32| rd(slot, map::DMEM + offset);
        let (a, b) = (-16_i32, 3_i32);
        let expect = [
            (0, (a - b) as u32),
            (4, 1),
            (8, 0),
            (12, (a >> b) as u32),
            (16, (a as u32) >> b),
            (20, 3 << 3),
            (24, (a ^ b) as u32),
            (28, (a | b) as u32),
            (32, (a & b) as u32),
            (36, (a >> 1) as u32),
            (40, 1),
            (44, 0x1234_5678),
            (48, 0x5678_0078),
            (52, a as u32),
            (56, a as u32),
            (60, 0xF0),
            (64, a as u32),
            (68, 0xFFF0),
            (72, jal_at + 4),
            (88, 0),
            (76, 2),
            (80, auipc_at + 0x1000),
            (84, link_at + 8),
            (92, !(a as u32)),
        ];
        for (offset, value) in expect {
            assert_eq!(dmem(&mut slot, offset), value, "DMEM +{offset}");
        }
        assert_eq!(rd(&mut slot, map::STATUS) & 0b100, 0, "no illegal instruction");
    }

    #[test]
    fn counter_counts_and_is_traced() {
        let mut slot = FpgaSlot::default();
        assert!(slot.add_spec(riscv_rv32i()).is_ok());
        for (i, w) in counter().iter().enumerate() {
            wr(&mut slot, map::IMEM + 4 * i as u32, *w);
        }
        slot.set_tracing(true);
        wr(&mut slot, map::CTRL, map::RESET_AND_RUN);
        slot.tick(1024 * 100);
        let out = rd(&mut slot, map::MAILBOX_OUT);
        assert!((30..=34).contains(&out), "about 100 clocks / 3 per loop, got {out}");
        let trace = slot.blocks()[0].core.trace().expect("tracing");
        let pc = trace.signals.iter().position(|s| s.name == "pc").expect("pc is traced");
        assert!(trace.signals[pc].changes.len() > 90, "pc changes every clock");
        assert_eq!(trace.value_at(pc, trace.now).map(|v| v % 4), Some(0));
    }

    #[test]
    fn illegal_instructions_halt() {
        let mut slot = run(&[addi(1, 0, 1), 0xFFFF_FFFF]);
        assert_eq!(rd(&mut slot, map::STATUS) & 0b110, 0b110);
        assert_eq!(rd(&mut slot, map::PC), 4);
    }

    #[test]
    fn spec_lists_the_register_map() {
        let spec = riscv_rv32i();
        assert_eq!(spec.name, "riscv_rv32i");
        assert_eq!((spec.size, spec.clock_div), (0x4000, 1024));
        let names: Vec<&str> = spec.registers.iter().map(|r| r.name.as_str()).collect();
        assert!(names.starts_with(&["CTRL", "STATUS", "PC"]), "{names:?}");
        assert!(spec.registers[0].description.contains("run"));
    }
}

#[cfg(test)]
mod bench {
    use ds_emu::slot2::{SLOT2_START, Slot2Device, Width};

    use super::{riscv_map as map, rv32::*, *};
    use crate::slot::FpgaSlot;

    /// `cargo test --release -p ds_fpga clock_rate -- --ignored --nocapture`
    #[test]
    #[ignore = "timing"]
    fn clock_rate() {
        let mut slot = FpgaSlot::default();
        assert!(slot.add_spec(riscv_rv32i()).is_ok());
        // An endless counting loop.
        for (i, w) in [addi(1, 1, 1), jal(0, -4)].iter().enumerate() {
            slot.write(SLOT2_START + map::IMEM + 4 * i as u32, *w, Width::Word);
        }
        slot.write(SLOT2_START + map::CTRL, map::RESET_AND_RUN, Width::Word);
        let start = std::time::Instant::now();
        slot.tick(1024 * 200_000);
        let secs = start.elapsed().as_secs_f64();
        println!("{:.0} RISC-V clocks/s", 200_000.0 / secs);
    }
}
