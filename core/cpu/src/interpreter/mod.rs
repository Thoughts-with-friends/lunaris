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

/// Decodes an ARM instruction into an enum
pub const fn arm_decode(instruction: u32) -> ARMInstr {
    // Here be dragons

    // Branch
    if ((instruction & 0x0F00_0000) >> 24) == 0xA {
        return ARMInstr::Branch;
    }

    // Branch with link
    if ((instruction & 0x0F00_0000) >> 24) == 0xB {
        return ARMInstr::BranchWithLink;
    }

    // BX
    if ((instruction >> 4) & 0x00FF_FFFF) == 0x12FFF1 {
        return ARMInstr::BranchExchange;
    }

    // BLX
    if ((instruction >> 4) & 0x00FF_FFFF) == 0x12FFF3 {
        return ARMInstr::BranchLinkExchange;
    }

    // ARMv5TE-only instructions
    if ((instruction >> 16) & 0xFFF) == 0x16F && ((instruction >> 4) & 0xFF) == 0xF1 {
        return ARMInstr::CountLeadingZeros;
    }

    // Saturated ops
    if ((instruction >> 4) & 0xFF) == 0x05 {
        if ((instruction >> 24) & 0xF) == 0x1 {
            let op = (instruction >> 20) & 0xF;
            if matches!(op, 0 | 2 | 4 | 6) {
                return ARMInstr::SaturatedOp;
            }
        }
    }

    // Data processing / multiply / halfword hell
    if ((instruction >> 26) & 0x3) == 0 {
        if (instruction & (1 << 25)) == 0 {
            // SWP
            if ((instruction >> 4) & 0xFF) == 0x9 {
                if ((instruction >> 23) & 0x1F) == 0x2 && ((instruction >> 20) & 0x3) == 0 {
                    return ARMInstr::Swap;
                }
            }

            // Signed halfword multiply
            if (instruction & (1 << 7)) != 0 && (instruction & (1 << 4)) == 0 {
                if (instruction & (1 << 20)) == 0 && ((instruction >> 23) & 0x3) == 0x2 {
                    return ARMInstr::SignedHalfwordMultiply;
                }
            }

            // Multiply / halfword transfers
            if (instruction & (1 << 7)) != 0 && (instruction & (1 << 4)) != 0 {
                if ((instruction >> 4) & 0xF) == 0x9 {
                    if ((instruction >> 22) & 0x3F) == 0 {
                        return ARMInstr::Multiply;
                    } else if ((instruction >> 23) & 0x1F) == 1 {
                        return ARMInstr::MultiplyLong;
                    }
                    return ARMInstr::Undefined;
                } else if (instruction & (1 << 6)) == 0 && (instruction & (1 << 5)) != 0 {
                    if (instruction & (1 << 20)) != 0 {
                        return ARMInstr::LoadHalfword;
                    } else {
                        return ARMInstr::StoreHalfword;
                    }
                } else if (instruction & (1 << 6)) != 0 && (instruction & (1 << 5)) == 0 {
                    if (instruction & (1 << 20)) != 0 {
                        return ARMInstr::LoadSignedByte;
                    } else {
                        return ARMInstr::LoadDoubleword;
                    }
                } else if (instruction & (1 << 6)) != 0 && (instruction & (1 << 5)) != 0 {
                    if (instruction & (1 << 20)) != 0 {
                        return ARMInstr::LoadSignedHalfword;
                    } else {
                        return ARMInstr::StoreDoubleword;
                    }
                }

                return ARMInstr::Undefined;
            }
        }

        return ARMInstr::DataProcessing;
    }

    // Single data transfer
    if ((instruction & 0x0F00_0000) >> 26) == 0x1 {
        if (instruction & (1 << 20)) == 0 {
            if (instruction & (1 << 22)) == 0 {
                return ARMInstr::StoreWord;
            } else {
                return ARMInstr::StoreByte;
            }
        } else {
            if (instruction & (1 << 22)) == 0 {
                return ARMInstr::LoadWord;
            } else {
                return ARMInstr::LoadByte;
            }
        }
    }

    // Block transfer
    if ((instruction >> 25) & 0x7) == 0x4 {
        if (instruction & (1 << 20)) == 0 {
            return ARMInstr::StoreBlock;
        } else {
            return ARMInstr::LoadBlock;
        }
    }

    // Coprocessor
    if ((instruction >> 24) & 0xF) == 0xE {
        if (instruction & (1 << 4)) != 0 {
            return ARMInstr::CopRegTransfer;
        } else {
            return ARMInstr::CopDataOp;
        }
    }

    // SWI
    if ((instruction >> 24) & 0xF) == 0xF {
        return ARMInstr::Swi;
    }

    ARMInstr::Undefined
}
