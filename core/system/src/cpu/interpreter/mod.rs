// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! cpuinsters.hpp
//!
pub mod arm_instruction;
pub mod thumb_instruction;

use ds_cpu::arm_cpu::CpuType;

use self::arm_instruction::blx;
use crate::{cpu::arm_table, emulator::Emulator};

/// Decodes and runs the 32-bit ARM instruction just fetched.
///
/// ```text
/// from: execute() step 5 (CPSR.T clear)
///   |
///   v
/// arm_interpret(emu, cpu)
///   1. instruction = current_instr, condition = bits 28-31
///   2. (config.test) log disassembly
///   3. ARM9 BLX imm (cond 0xF, 0xFA/0xFB) -> blx()
///      otherwise: condition passes? -> ARM_TABLE[bits 20-27:4-7]
///                 (data processing, multiply, load/store, block
///                 transfer, branch, coprocessor, SWI...)
///   |
///   v
/// to:   the handler updates registers / memory through read_* / write_*
/// ```
pub fn arm_interpret(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u32 = emu.get_cpu_mut(cpu_type).get_current_instr();
    // ARM CPU Flags & Condition Field (cond): https://problemkaputt.de/gbatek.htm#armcpuflagsconditionfieldcond
    let condition: u32 = (instruction & 0xF000_0000) >> 28; // condition field (bits 28-31)

    // In ARM, PC reads as current + 8
    let cpu_id = emu.get_cpu(cpu_type).get_id();
    let test_mode = emu.config.test;

    // Debug
    if cpu_id <= 0 && test_mode {
        if cpu_id <= 0 {
            #[cfg(feature = "log")]
            log::trace!("(9A)");
        } else {
            #[cfg(feature = "log")]
            log::trace!("(7A)");
        }
        let pc: u32 = emu.get_cpu(cpu_type).get_pc().wrapping_sub(8);

        #[cfg(feature = "log")]
        log::trace!("[{pc:08X}] {instruction:08X} - ");

        let cpu = emu.get_cpu_mut(cpu_type);

        #[cfg(feature = "log")]
        let disasm = ds_cpu::disassemble::disasm_arm(cpu, instruction, pc);
        #[cfg(feature = "log")]
        log::trace!("disasm: {disasm}");
    }

    // Build opcode
    let op: u32 = ((instruction >> 4) & 0xF) | ((instruction >> 16) & 0xFF0); // table index = bits 4-7 | bits 20-27 << 4

    // Special BLX handling
    match condition == 15 && (instruction & 0xFE00_0000) == 0xFA00_0000 && cpu_id <= 0 {
        // cond=1111b with bits 25-27 = 101b: BLX imm (ARMv5, ARM9 only)
        true => blx(emu, cpu_type, instruction),
        false => {
            if emu.get_cpu_mut(cpu_type).check_condition(condition as i32) {
                // Assuming arm_table is indexed by u32 and stores fn(&mut Cpu, u32)
                arm_table::ARM_TABLE[op as usize](emu, cpu_type, instruction);
            }
        }
    }
}
