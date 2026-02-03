// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! cpuinsters.hpp
//!
pub mod arm_instruction;
pub mod thumb_instruction;

use self::arm_instruction::blx;
use crate::arm_cpu::ArmCpu;
use crate::arm_table;
use crate::instruction_table::ARMInstr;

/// Interprets an ARM instruction
pub fn arm_interpret(cpu: &mut ArmCpu) {
    let instruction: u32 = cpu.get_current_instr();
    let condition: u32 = (instruction & 0xF000_0000) >> 28;

    // In ARM, PC reads as current + 8
    let pc: u32 = cpu.get_pc().wrapping_sub(8);

    let cpu_id = cpu.get_id();

    // if cpu_id <= 0 && Config::test {
    //     if cpu_id <= 0 {
    //         print!("(9A)");
    //     } else {
    //         print!("(7A)");
    //     }

    //     print!("[$%08X] {{$%08X}} - ", pc, instruction);

    //     let disasm = Disassembler::disasm_arm(cpu, instruction, pc);
    //     print!(" {}", disasm);
    //     println!();
    // }

    // Build opcode
    let op: u32 = ((instruction >> 4) & 0xF) | ((instruction >> 16) & 0xFF0);

    // Special BLX handling
    match condition == 15 && (instruction & 0xFE00_0000) == 0xFA00_0000 && cpu_id <= 0 {
        true => {
            blx(cpu, instruction);
        }
        false => {
            if cpu.check_condition(condition as i32) {
                // Assuming arm_table is indexed by u32 and stores fn(&mut Cpu, u32)
                arm_table::ARM_TABLE[op as usize](cpu, instruction);
            }
        }
    }
}

/// Decodes an ARM instruction into an enum or struct
pub fn arm_decode(instruction: u32) -> ARMInstr {
    todo!()
}
