// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! instrthumb.cpp
//!

use crate::cpu::arm_cpu::{CpuType, REG_LR, REG_PC, REG_SP};
use crate::cpu::instruction_table::ThumbInstr;
use crate::emulator::Emulator;

/// Interprets a Thumb instruction
pub fn thumb_interpret(emu: &mut Emulator, cpu_type: CpuType) {
    // NOTE: tracing output is gated behind the `tracing` feature
    let instruction = emu.get_cpu_mut(cpu_type).get_current_instr() & 0xFFFF;
    let cpu_id = emu.get_cpu(cpu_type).get_id();

    if cpu_id > 0 {
        #[cfg(feature = "tracing")]
        {
            match !cpu_id > 0 {
                true => tracing::error!("(9T)"),
                false => tracing::error!("(7T)"),
            }

            tracing::error!(
                " ${:08X} - ",
                emu.get_cpu(cpu_type).get_pc().wrapping_sub(4)
            );
            tracing::error!("(${:#06X}) ", instruction);
        }
    }

    // This value is only referenced by the Thumb decode.
    let opcode = thumb_decode(instruction);

    match opcode {
        ThumbInstr::MovShift => thumb_mov_shift(emu, cpu_type),
        ThumbInstr::AddReg => thumb_add_reg(emu, cpu_type),
        ThumbInstr::SubReg => thumb_sub_reg(emu, cpu_type),
        ThumbInstr::MovImm => thumb_mov(emu, cpu_type),
        ThumbInstr::CmpImm => thumb_cmp(emu, cpu_type),
        ThumbInstr::AddImm => thumb_add(emu, cpu_type),
        ThumbInstr::SubImm => thumb_sub(emu, cpu_type),
        ThumbInstr::AluOp => thumb_alu_op(emu, cpu_type),
        ThumbInstr::HiRegOp => thumb_hi_reg_op(emu, cpu_type),
        ThumbInstr::PcRelLoad => thumb_pc_rel_load(emu, cpu_type),
        ThumbInstr::StoreImmOffset => thumb_store_imm_offset(emu, cpu_type),
        ThumbInstr::LoadImmOffset => thumb_load_imm_offset(emu, cpu_type),
        ThumbInstr::StoreRegOffset => thumb_store_reg_offset(emu, cpu_type),
        ThumbInstr::LoadRegOffset => thumb_load_reg_offset(emu, cpu_type),
        ThumbInstr::StoreHalfword => thumb_store_halfword(emu, cpu_type),
        ThumbInstr::LoadHalfword => thumb_load_halfword(emu, cpu_type),
        ThumbInstr::LoadStoreSignHalfword => thumb_load_store_sign_halfword(emu, cpu_type),
        ThumbInstr::SpRelStore => thumb_sp_rel_store(emu, cpu_type),
        ThumbInstr::SpRelLoad => thumb_sp_rel_load(emu, cpu_type),
        ThumbInstr::OffsetSp => thumb_offset_sp(emu, cpu_type),
        ThumbInstr::LoadAddress => thumb_load_address(emu, cpu_type),
        ThumbInstr::LoadMultiple => thumb_load_multiple(emu, cpu_type),
        ThumbInstr::StoreMultiple => thumb_store_multiple(emu, cpu_type),
        ThumbInstr::Push => thumb_push(emu, cpu_type),
        ThumbInstr::Pop => thumb_pop(emu, cpu_type),
        ThumbInstr::Branch => thumb_branch(emu, cpu_type),
        ThumbInstr::CondBranch => thumb_cond_branch(emu, cpu_type),
        ThumbInstr::LongBranchPrep => thumb_long_branch_prep(emu, cpu_type),
        ThumbInstr::LongBranch => thumb_long_branch(emu, cpu_type),
        ThumbInstr::LongBlx => thumb_long_blx(emu, cpu_type),

        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!(
                "Unrecognized Thumb opcode ${:04X}",
                emu.get_cpu_mut(cpu_type).get_current_instr() & 0xFFFF
            );
            return;
        }
    }

    if cpu_id > 0 {
        #[cfg(feature = "tracing")]
        tracing::error!("");
    }
}

/// Decodes a Thumb instruction into an enum or struct
#[allow(clippy::missing_const_for_fn)]
pub fn thumb_decode(instruction: u32) -> ThumbInstr {
    let instr13 = instruction >> 13;
    let instr12 = instruction >> 12;
    let instr11 = instruction >> 11;
    let instr10 = instruction >> 10;

    match instr11 {
        0x4 => return ThumbInstr::MovImm,
        0x5 => return ThumbInstr::CmpImm,
        0x6 => return ThumbInstr::AddImm,
        0x7 => return ThumbInstr::SubImm,
        0x9 => return ThumbInstr::PcRelLoad,
        0x10 => return ThumbInstr::StoreHalfword,
        0x11 => return ThumbInstr::LoadHalfword,
        0x12 => return ThumbInstr::SpRelStore,
        0x13 => return ThumbInstr::SpRelLoad,
        0x18 => return ThumbInstr::StoreMultiple,
        0x19 => return ThumbInstr::LoadMultiple,
        0x1C => return ThumbInstr::Branch,
        0x1D => return ThumbInstr::LongBlx,
        _ => {}
    }

    if instr13 == 0 {
        if (instr11 & 0x3) != 0x3 {
            return ThumbInstr::MovShift;
        } else {
            if (instruction & (1 << 9)) != 0 {
                return ThumbInstr::SubReg;
            }
            return ThumbInstr::AddReg;
        }
    }

    match instr10 {
        0x10 => return ThumbInstr::AluOp,
        0x11 => return ThumbInstr::HiRegOp,
        _ => {}
    }

    match instr12 {
        0x5 => {
            if (instruction & (1 << 9)) == 0 {
                if (instruction & (1 << 11)) == 0 {
                    return ThumbInstr::StoreRegOffset;
                }
                return ThumbInstr::LoadRegOffset;
            }
            return ThumbInstr::LoadStoreSignHalfword;
        }
        0xA => return ThumbInstr::LoadAddress,
        0xB => {
            if ((instruction >> 9) & 0x3) == 0x2 {
                if (instruction & (1 << 11)) != 0 {
                    return ThumbInstr::Pop;
                }
                return ThumbInstr::Push;
            }
            return ThumbInstr::OffsetSp;
        }
        0xD => return ThumbInstr::CondBranch,
        0xF => {
            if (instruction & (1 << 11)) == 0 {
                return ThumbInstr::LongBranchPrep;
            }
            return ThumbInstr::LongBranch;
        }
        _ => {}
    }

    if instr13 == 0x3 {
        if (instruction & (1 << 11)) == 0 {
            return ThumbInstr::StoreImmOffset;
        }
        return ThumbInstr::LoadImmOffset;
    }

    ThumbInstr::Undefined
}

/// Thumb instruction: MOV with shift
#[allow(clippy::missing_const_for_fn)]
pub fn thumb_mov_shift(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;
    let opcode = ((instruction >> 11) & 0x3) as u32;
    let mut shift = ((instruction >> 6) & 0x1F) as u32;
    let source = ((instruction >> 3) & 0x7) as u32;
    let destination = (instruction & 0x7) as u32;

    let mut value: u32 = emu.get_cpu_mut(cpu_type).get_register(source as i32);

    match opcode {
        0 => {
            value = emu.get_cpu_mut(cpu_type).lsl(value, shift as i32, true);
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("LSL {{{}}}, {{{}}}, #{}", destination, source, shift);
            // }
        }
        1 => {
            if shift == 0 {
                shift = 32;
            }
            value = emu.get_cpu_mut(cpu_type).lsr(value, shift as i32, true);
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("LSR {{{}}}, {{{}}}, #{}", destination, source, shift);
            // }
        }
        2 => {
            if shift == 0 {
                shift = 32;
            }
            value = emu.get_cpu_mut(cpu_type).asr(value, shift as i32, true);
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("ASR {{{}}}, {{{}}}, #{}", destination, source, shift);
            // }
        }
        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!("Unrecognized opcode {opcode} in thumb_mov_shift");
        }
    }

    emu.get_cpu_mut(cpu_type).add_internal_cycles(1); // Extra cycle due to register shift
    emu.get_cpu_mut(cpu_type)
        .set_register(destination as i32, value);
    // Optionally: emu.get_cpu_mut(cpu_type).set_lo_register(destination, value);
}

/// Thumb instruction: ADD register
pub fn thumb_add_reg(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;
    let destination = (instruction & 0x7) as u32;
    let source = ((instruction >> 3) & 0x7) as u32;
    let mut operand = ((instruction >> 6) & 0x7) as u32;
    let is_imm = (instruction & (1 << 10)) != 0;

    if !is_imm {
        // if emu.get_cpu(cpu_type).get_id() > 0 {
        //     // println!("ADD {{{}}}, {{{}}}, {{{}}}", destination, source, operand);
        // }
        operand = emu.get_cpu_mut(cpu_type).get_register(operand as i32);
    }
    // else if emu.get_cpu(cpu_type).get_id() > 0 {
    //     // println!("ADD {{{}}}, {{{}}}, ${:08X}", destination, source, operand);
    // }
    let src = emu.get_cpu_mut(cpu_type).get_register(source as i32);
    emu.get_cpu_mut(cpu_type)
        .add(destination, src, operand, true);
}

/// Thumb instruction: SUB register
pub fn thumb_sub_reg(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;
    let destination = (instruction & 0x7) as u32;
    let source = ((instruction >> 3) & 0x7) as u32;
    let mut operand = ((instruction >> 6) & 0x7) as u32;
    let is_imm = (instruction & (1 << 10)) != 0;

    if !is_imm {
        // if emu.get_cpu(cpu_type).get_id() > 0 {
        //     // println!("SUB {{{}}}, {{{}}}, {{{}}}", destination, source, operand);
        // }
        operand = emu.get_cpu(cpu_type).get_register(operand as i32);
    } else if emu.get_cpu(cpu_type).get_id() > 0 {
        // println!("SUB {{{}}}, {{{}}}, ${:08X}", destination, source, operand);
    }

    let src = emu.get_cpu(cpu_type).get_register(source as i32);
    emu.get_cpu_mut(cpu_type)
        .sub(destination, src, operand, true);
}

/// Thumb instruction: MOV immediate
pub fn thumb_mov(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;
    let offset = (instruction & 0xFF) as u32;
    let reg = ((instruction >> 8) & 0x7) as u32;

    // if emu.get_cpu(cpu_type).get_id() > 0 {
    //     // println!("MOV {{{}}}, ${:02X}", reg, offset);
    // }

    emu.get_cpu_mut(cpu_type).mov(reg, offset, true);
}

/// Thumb instruction: CMP
pub fn thumb_cmp(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;
    let offset: u32 = (instruction & 0xFF) as u32;
    let reg: u32 = ((instruction >> 8) & 0x7) as u32;

    // if emu.get_cpu(cpu_type).get_id() > 0 {
    //     // println!("CMP {{{}}}, ${:02X}", reg, offset);
    // }

    let x = emu.get_cpu(cpu_type).get_register(reg as i32);
    emu.get_cpu_mut(cpu_type).cmp(x, offset);
}

/// Thumb instruction: ADD
pub fn thumb_add(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;
    let offset: u32 = (instruction & 0xFF) as u32;
    let reg: u32 = ((instruction >> 8) & 0x7) as u32;

    // if emu.get_cpu(cpu_type).get_id() > 0 {
    //     // println!("ADD {{{}}}, ${:02X}", reg, offset);
    // }

    let src = emu.get_cpu_mut(cpu_type).get_register(reg as i32);
    emu.get_cpu_mut(cpu_type).add(reg, src, offset, true);
}

/// Thumb instruction: SUB
pub fn thumb_sub(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;
    let offset: u32 = (instruction & 0xFF) as u32;
    let reg: u32 = ((instruction >> 8) & 0x7) as u32;

    // if emu.get_cpu(cpu_type).get_id() > 0 {
    //     // println!("SUB {{{}}}, ${:02X}", reg, offset);
    // }
    let src = emu.get_cpu_mut(cpu_type).get_register(reg as i32);
    emu.get_cpu_mut(cpu_type).sub(reg, src, offset, true);
}

/// Thumb instruction: ALU operations
#[allow(clippy::missing_const_for_fn)]
pub fn thumb_alu_op(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;

    let destination = (instruction & 0x7) as u32;
    let source = ((instruction >> 3) & 0x7) as u32;
    let opcode = ((instruction >> 6) & 0xF) as u32;

    match opcode {
        0x0 => {
            // AND
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("AND {{{}}}, {{{}}}", destination, source);
            // }
            let src = emu.get_cpu_mut(cpu_type).get_register(destination as i32) as i32;
            let operand = emu.get_cpu_mut(cpu_type).get_register(source as i32) as i32;
            emu.get_cpu_mut(cpu_type)
                .andd(destination as i32, src, operand, true);
        }
        0x1 => {
            // EOR
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("EOR {{{}}}, {{{}}}", destination, source);
            // }
            let src = emu.get_cpu_mut(cpu_type).get_register(destination as i32) as i32;
            let operand = emu.get_cpu_mut(cpu_type).get_register(source as i32) as i32;
            emu.get_cpu_mut(cpu_type)
                .eor(destination as i32, src, operand, true);
        }
        0x2 => {
            // LSL
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("LSL {{{}}}, {{{}}}", destination, source);
            // }
            let mut reg = emu.get_cpu_mut(cpu_type).get_register(destination as i32);
            let shift = emu.get_cpu_mut(cpu_type).get_register(source as i32) as i32;
            reg = emu.get_cpu_mut(cpu_type).lsl(reg, shift, true);
            emu.get_cpu_mut(cpu_type)
                .set_register(destination as i32, reg);
        }
        0x3 => {
            // LSR
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("LSR {{{}}}, {{{}}}", destination, source);
            // }
            let mut reg = emu.get_cpu_mut(cpu_type).get_register(destination as i32);
            let shift = emu.get_cpu_mut(cpu_type).get_register(source as i32) as i32;
            reg = emu.get_cpu_mut(cpu_type).lsr(reg, shift, true);
            emu.get_cpu_mut(cpu_type)
                .set_register(destination as i32, reg);
        }
        0x4 => {
            // ASR
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("ASR {{{}}}, {{{}}}", destination, source);
            // }
            let mut reg = emu.get_cpu_mut(cpu_type).get_register(destination as i32);
            let shift = emu.get_cpu_mut(cpu_type).get_register(source as i32) as i32;
            reg = emu.get_cpu_mut(cpu_type).asr(reg, shift, true);
            emu.get_cpu_mut(cpu_type)
                .set_register(destination as i32, reg);
        }
        0x5 => {
            // ADC
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("ADC {{{}}}, {{{}}}", destination, source);
            // }
            let src = emu.get_cpu_mut(cpu_type).get_register(destination as i32);
            let operand = emu.get_cpu_mut(cpu_type).get_register(source as i32);
            emu.get_cpu_mut(cpu_type)
                .adc(destination, src, operand, true);
        }
        0x6 => {
            // SBC
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("SBC {{{}}}, {{{}}}", destination, source);
            // }
            let src = emu.get_cpu_mut(cpu_type).get_register(destination as i32);
            let operand = emu.get_cpu_mut(cpu_type).get_register(source as i32);
            emu.get_cpu_mut(cpu_type)
                .sbc(destination, src, operand, true);
        }
        0x7 => {
            // ROR
            if emu.get_cpu(cpu_type).get_id() > 0 {
                // println!("ROR {{{}}}, {{{}}}", destination, source);
            }
            let mut reg = emu.get_cpu(cpu_type).get_register(destination as i32);
            let c = emu.get_cpu(cpu_type).get_register(source as i32);
            reg = emu.get_cpu_mut(cpu_type).rotr32(reg, c, true);
            emu.get_cpu_mut(cpu_type)
                .set_register(destination as i32, reg);
        }
        0x8 => {
            // TST
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("TST {{{}}}, {{{}}}", destination, source);
            // }
            let x = emu.get_cpu_mut(cpu_type).get_register(destination as i32);
            let y = emu.get_cpu_mut(cpu_type).get_register(source as i32);
            emu.get_cpu_mut(cpu_type).tst(x, y);
        }
        0x9 => {
            // NEG
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("NEG {{{}}}, {{{}}}", destination, source);
            // }

            let operand = emu.get_cpu_mut(cpu_type).get_register(source as i32);
            emu.get_cpu_mut(cpu_type).sub(destination, 0, operand, true);
        }
        0xA => {
            // CMP
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("CMP {{{}}}, {{{}}}", destination, source);
            // }
            let x = emu.get_cpu_mut(cpu_type).get_register(destination as i32);
            let y = emu.get_cpu_mut(cpu_type).get_register(source as i32);
            emu.get_cpu_mut(cpu_type).cmp(x, y);
        }
        0xB => {
            // CMN
            if emu.get_cpu(cpu_type).get_id() > 0 {
                // println!("CMN {{{}}}, {{{}}}", destination, source);
            }
            let x = emu.get_cpu_mut(cpu_type).get_register(destination as i32);
            let y = emu.get_cpu_mut(cpu_type).get_register(source as i32);
            emu.get_cpu_mut(cpu_type).cmn(x, y);
        }
        0xC => {
            // ORR
            if emu.get_cpu(cpu_type).get_id() > 0 {
                // println!("ORR {{{}}}, {{{}}}", destination, source);
            }
            let reg = emu.get_cpu_mut(cpu_type).get_register(destination as i32) as i32;
            let src = emu.get_cpu_mut(cpu_type).get_register(source as i32) as i32;
            emu.get_cpu_mut(cpu_type)
                .orr(destination as i32, reg, src, true);
        }
        0xD => {
            // MUL
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("MUL {{{}}}, {{{}}}", destination, source);
            // }
            let src = emu.get_cpu_mut(cpu_type).get_register(destination as i32);
            let operand = emu.get_cpu_mut(cpu_type).get_register(source as i32);
            emu.get_cpu_mut(cpu_type)
                .mul(destination, src, operand, true);
            if !emu.get_cpu(cpu_type).get_id() > 0 {
                emu.get_cpu_mut(cpu_type).add_internal_cycles(3);
            } else {
                let multiplicand = emu.get_cpu_mut(cpu_type).get_register(destination as i32);
                if multiplicand & 0xFF000000 != 0 {
                    emu.get_cpu_mut(cpu_type).add_internal_cycles(4);
                } else if multiplicand & 0x00FF0000 != 0 {
                    emu.get_cpu_mut(cpu_type).add_internal_cycles(3);
                } else if multiplicand & 0x0000FF00 != 0 {
                    emu.get_cpu_mut(cpu_type).add_internal_cycles(2);
                } else {
                    emu.get_cpu_mut(cpu_type).add_internal_cycles(1);
                }
            }
        }
        0xE => {
            // BIC
            if emu.get_cpu(cpu_type).get_id() > 0 {
                // println!("BIC {{{}}}, {{{}}}", destination, source);
            }
            let src = emu.get_cpu_mut(cpu_type).get_register(destination as i32);
            let operand = emu.get_cpu_mut(cpu_type).get_register(source as i32);
            emu.get_cpu_mut(cpu_type)
                .bic(destination, src, operand, true);
        }
        0xF => {
            // MVN
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("MVN {{{}}}, {{{}}}", destination, source);
            // }

            let operand = emu.get_cpu(cpu_type).get_register(source as i32);
            emu.get_cpu_mut(cpu_type).mvn(destination, operand, true);
        }
        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!("Invalid thumb alu op {opcode}");
        }
    }
}

/// Thumb instruction: High register operations
#[allow(clippy::missing_const_for_fn)]
pub fn thumb_hi_reg_op(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;

    let opcode: u32 = ((instruction >> 8) & 0x3) as u32;
    let high_source: bool = (instruction & (1 << 6)) != 0;
    let high_dest: bool = (instruction & (1 << 7)) != 0;

    let mut source: u32 = ((instruction >> 3) & 0x7) as u32;
    if high_source {
        source |= 8;
    }

    let mut destination: u32 = (instruction & 0x7) as u32;
    if high_dest {
        destination |= 8;
    }

    match opcode {
        0x0 => {
            // ADD
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("ADD {{{}}}, {{{}}}", destination, source);
            // }
            if destination == REG_PC {
                let reg = emu.get_cpu(cpu_type).get_register(source as i32);
                let new_addr = emu.get_cpu(cpu_type).get_pc().wrapping_add(reg);
                emu.get_cpu_mut(cpu_type).jp(new_addr, false);
            } else {
                let src = emu.get_cpu(cpu_type).get_register(destination as i32);
                let operand = emu.get_cpu(cpu_type).get_register(source as i32);
                emu.get_cpu_mut(cpu_type)
                    .add(destination, src, operand, false);
            }
        }
        0x1 => {
            // CMP
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("CMP {{{}}}, {{{}}}", destination, source);
            // }
            let x = emu.get_cpu(cpu_type).get_register(destination as i32);
            let y = emu.get_cpu(cpu_type).get_register(source as i32);
            emu.get_cpu_mut(cpu_type).cmp(x, y);
        }
        0x2 => {
            // MOV
            // if emu.get_cpu(cpu_type).get_id() > 0 {
            //     // println!("MOV {{{}}}, {{{}}}", destination, source);
            // }
            if destination == REG_PC {
                let new_addr = emu.get_cpu(cpu_type).get_register(source as i32);
                emu.get_cpu_mut(cpu_type).jp(new_addr, false);
            } else {
                let operand = emu.get_cpu(cpu_type).get_register(source as i32);
                emu.get_cpu_mut(cpu_type).mov(destination, operand, false);
            }
        }
        0x3 => {
            // BX / BLX
            if high_dest {
                // if emu.get_cpu(cpu_type).get_id() > 0 {
                //     // println!("BLX {{{}}}", source);
                // }
                let value = emu.get_cpu(cpu_type).get_pc().wrapping_sub(1);
                emu.get_cpu_mut(cpu_type).set_register(REG_LR as i32, value);
            } else if emu.get_cpu(cpu_type).get_id() > 0 {
                #[cfg(feature = "tracing")]
                tracing::error!("BX {{{source}}}");
            }
            let new_addr = emu.get_cpu(cpu_type).get_register(source as i32);
            emu.get_cpu_mut(cpu_type).jp(new_addr, true);
        }
        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!("High-reg Thumb opcode ${opcode:02X} not recognized");
        }
    }
}

/// Thumb instruction: PC-relative load
pub fn thumb_pc_rel_load(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;

    let destination: u32 = ((instruction >> 8) & 0x7) as u32;
    let mut address: u32 = emu.get_cpu(cpu_type).get_pc();
    address = address.wrapping_add(((instruction & 0xFF) as u32) << 2);
    address &= !0x3; // 4-byte alignment

    // if emu.get_cpu(cpu_type).get_id() > 0 {
    //     // println!("LDR {{{}}}, ${:08X}", destination, address);
    // }

    emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);
    emu.get_cpu_mut(cpu_type).add_internal_cycles(1);
    let value = emu.read_word(address, cpu_type);
    emu.get_cpu_mut(cpu_type)
        .set_register(destination as i32, value);
}

/// Thumb instruction: Store register with offset
pub fn thumb_store_reg_offset(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;

    let is_byte: bool = (instruction & (1 << 10)) != 0;
    let base: u32 = ((instruction >> 3) & 0x7) as u32;
    let source: u32 = (instruction & 0x7) as u32;
    let offset: u32 = ((instruction >> 6) & 0x7) as u32;

    let mut address: u32 = emu.get_cpu(cpu_type).get_register(base as i32);
    address = address.wrapping_add(emu.get_cpu(cpu_type).get_register(offset as i32));

    let source_contents: u32 = emu.get_cpu(cpu_type).get_register(source as i32);

    if is_byte {
        // if emu.get_cpu(cpu_type).get_id() > 0 {
        //     // println!("STRB {{{}}}, [{{{}}}, {{{}}}]", source, base, offset);
        // }
        emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
        emu.write_byte(address, (source_contents & 0xFF) as u8, cpu_type);
    } else {
        if emu.get_cpu(cpu_type).get_id() > 0 {
            // println!("STR {{{}}}, [{{{}}}, {{{}}}]", source, base, offset);
        }
        emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);
        emu.write_word(address, source_contents, cpu_type);
    }
}

/// Thumb instruction: Load register with offset
pub fn thumb_load_reg_offset(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;

    let is_byte: bool = (instruction & (1 << 10)) != 0;
    let base = ((instruction >> 3) & 0x7) as u32;
    let destination = (instruction & 0x7) as u32;
    let offset = ((instruction >> 6) & 0x7) as u32;

    let mut address: u32 = emu.get_cpu(cpu_type).get_register(base as i32);
    address = address.wrapping_add(emu.get_cpu(cpu_type).get_register(offset as i32));

    if is_byte {
        // if emu.get_cpu(cpu_type).get_id() > 0 {
        //     // println!("LDRB {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
        // }
        emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
        emu.get_cpu_mut(cpu_type).add_internal_cycles(1);
        let value = emu.read_byte(address, cpu_type);
        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, value as u32);
    } else {
        if emu.get_cpu(cpu_type).get_id() > 0 {
            // println!("LDR {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
        }
        emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);
        emu.get_cpu_mut(cpu_type).add_internal_cycles(1);
        let aligned_addr = address & !0x3;
        let rotate = (address & 0x3) * 8;
        let addr = emu.read_word(aligned_addr, cpu_type);
        let word = emu.get_cpu_mut(cpu_type).rotr32(addr, rotate, false);
        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, word);
    }
}

/// Thumb instruction: Load halfword
pub fn thumb_load_halfword(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;

    let offset: u32 = (((instruction >> 6) & 0x1F) << 1) as u32;
    let base: u32 = ((instruction >> 3) & 0x7) as u32;
    let source: u32 = (instruction & 0x7) as u32;

    let address: u32 = emu
        .get_cpu_mut(cpu_type)
        .get_register(base as i32)
        .wrapping_add(offset);
    let value = (emu.get_cpu(cpu_type).get_register(source as i32) & 0xFFFF) as u16;

    if emu.get_cpu(cpu_type).get_id() > 0 {
        // println!("STRH {{{}}}, [{{{}}}, ${:04X}]", source, base, offset);
    }

    emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
    emu.write_halfword(address, value, cpu_type);
}

/// Thumb instruction: Store halfword
pub fn thumb_store_halfword(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;

    let offset: u32 = (((instruction >> 6) & 0x1F) << 1) as u32;
    let base = ((instruction >> 3) & 0x7) as u32;
    let destination = (instruction & 0x7) as u32;

    let address: u32 = emu
        .get_cpu(cpu_type)
        .get_register(base as i32)
        .wrapping_add(offset);

    if emu.get_cpu(cpu_type).get_id() > 0 {
        // println!("LDRH {{{}}}, [{{{}}}, ${:04X}]", destination, base, offset);
    }

    emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
    let value = emu.read_halfword(address, cpu_type);
    emu.get_cpu_mut(cpu_type)
        .set_register(destination as i32, value as u32);
}

/// Thumb instruction: Store with immediate offset
pub fn thumb_store_imm_offset(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;

    let source: u32 = (instruction & 0x7) as u32;
    let base: u32 = ((instruction >> 3) & 0x7) as u32;
    let mut offset: u32 = ((instruction >> 6) & 0x1F) as u32;
    let is_byte: bool = (instruction & (1 << 12)) != 0;

    let mut address: u32 = emu.get_cpu(cpu_type).get_register(base as i32);

    if is_byte {
        address = address.wrapping_add(offset);

        if emu.get_cpu(cpu_type).get_id() > 0 {
            // println!("STRB {{{}}}, [{{{}}}, ${:02X}]", source, base, offset);
        }

        emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
        let value = (emu.get_cpu_mut(cpu_type).get_register(source as i32) & 0xFF) as u8;
        emu.write_byte(address, value, cpu_type);
    } else {
        offset <<= 2;
        address = address.wrapping_add(offset);

        if emu.get_cpu(cpu_type).get_id() > 0 {
            // println!("STR {{{}}}, [{{{}}}, ${:02X}]", source, base, offset);
        }

        emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);
        let value = emu.get_cpu_mut(cpu_type).get_register(source as i32);
        emu.write_word(address, value, cpu_type);
    }
}

/// Thumb instruction: Load with immediate offset
pub fn thumb_load_imm_offset(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;

    let destination: u32 = (instruction & 0x7) as u32;
    let base: u32 = ((instruction >> 3) & 0x7) as u32;
    let mut offset: u32 = ((instruction >> 6) & 0x1F) as u32;
    let is_byte: bool = (instruction & (1 << 12)) != 0;

    let mut address: u32 = emu.get_cpu(cpu_type).get_register(base as i32);

    if is_byte {
        address = address.wrapping_add(offset);

        #[cfg(feature = "tracing")]
        if emu.get_cpu(cpu_type).get_id() > 0 {
            tracing::debug!("LDRB {{{}}}, [{{{}}}, ${:02X}]", destination, base, offset);
        }

        emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
        let value = emu.read_byte(address, cpu_type);
        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, value as u32);
    } else {
        offset <<= 2;
        address = address.wrapping_add(offset);

        if emu.get_cpu(cpu_type).get_id() > 0 {
            // println!("LDR {{{}}}, [{{{}}}, ${:02X}]", destination, base, offset);
        }

        emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);

        let aligned_addr = address & !0x3;
        let rotate = (address & 0x3) * 8;
        let addr = emu.read_word(aligned_addr, cpu_type);
        let word = emu.get_cpu_mut(cpu_type).rotr32(addr, rotate, false);

        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, word);
    }

    emu.get_cpu_mut(cpu_type).add_internal_cycles(1);
}

/// Thumb instruction: Load/store signed halfword
pub fn thumb_load_store_sign_halfword(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;

    let destination: u32 = (instruction & 0x7) as u32;
    let base: u32 = ((instruction >> 3) & 0x7) as u32;
    let offset: u32 = ((instruction >> 6) & 0x7) as u32;
    let opcode: u32 = ((instruction >> 10) & 0x3) as u32;

    let mut address: u32 = emu.get_cpu(cpu_type).get_register(base as i32);
    address = address.wrapping_add(emu.get_cpu(cpu_type).get_register(offset as i32));

    match opcode {
        0 => {
            // STRH
            if emu.get_cpu(cpu_type).get_id() > 0 {
                // println!("STRH {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
            }
            let value = (emu.get_cpu(cpu_type).get_register(destination as i32) & 0xFFFF) as u16;
            emu.write_halfword(address, value, cpu_type);
            emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);
        }

        1 => {
            // LDSB
            if emu.get_cpu(cpu_type).get_id() > 0 {
                // println!("LDSB {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
            }
            let mut extended_byte: u32 = emu.read_byte(address, cpu_type).into();
            extended_byte = (extended_byte as i8 as i32) as u32;
            emu.get_cpu_mut(cpu_type)
                .set_register(destination as i32, extended_byte);
            emu.get_cpu_mut(cpu_type).add_internal_cycles(1);
            emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
        }

        2 => {
            // LDRH
            if emu.get_cpu(cpu_type).get_id() > 0 {
                // println!("LDRH {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
            }
            let value = emu.read_halfword(address, cpu_type);
            emu.get_cpu_mut(cpu_type)
                .set_register(destination as i32, value as u32);
            emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
            emu.get_cpu_mut(cpu_type).add_internal_cycles(1);
        }

        3 => {
            // LDSH
            if emu.get_cpu(cpu_type).get_id() > 0 {
                // println!("LDSH {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
            }
            let mut extended_halfword: u32 = emu.read_halfword(address, cpu_type).into();
            extended_halfword = (extended_halfword as i16 as i32) as u32;
            emu.get_cpu_mut(cpu_type)
                .set_register(destination as i32, extended_halfword);
            emu.get_cpu_mut(cpu_type).add_internal_cycles(1);
            emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
        }

        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!("Sign extended opcode {opcode} not recognized");
        }
    }
}

/// Thumb instruction: Stack pointer-relative store
pub fn thumb_sp_rel_store(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;

    let source: u32 = ((instruction >> 8) & 0x7) as u32;
    let offset: u32 = ((instruction & 0x00FF) as u32) << 2;

    let mut address: u32 = emu.get_cpu(cpu_type).get_register(REG_SP as i32);
    address = address.wrapping_add(offset);

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("STR {{{}}}, [SP, ${:04X}]", source, offset);
    // }

    emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);
    let value = emu.get_cpu(cpu_type).get_register(source as i32);
    emu.write_word(address, value, cpu_type);
}

/// Thumb instruction: Stack pointer-relative load
pub fn thumb_sp_rel_load(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;

    let destination: u32 = ((instruction >> 8) & 0x7) as u32;
    let offset: u32 = ((instruction & 0x00FF) as u32) << 2;

    let mut address: u32 = emu.get_cpu(cpu_type).get_register(REG_SP as i32);
    address = address.wrapping_add(offset);

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("LDR {{{}}}, [SP, ${:04X}]", destination, offset);
    // }

    emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);
    emu.get_cpu_mut(cpu_type).add_internal_cycles(1);

    let value = emu.read_word(address, cpu_type);
    emu.get_cpu_mut(cpu_type)
        .set_register(destination as i32, value);
}

/// Thumb instruction: Offset SP operation
pub fn thumb_offset_sp(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;

    let mut offset: i16 = ((instruction & 0x007F) << 2) as i16;

    if (instruction & (1 << 7)) != 0 {
        offset = -offset;
    }

    let sp: u32 = emu.get_cpu(cpu_type).get_register(REG_SP as i32);
    emu.get_cpu_mut(cpu_type)
        .set_register(REG_SP as i32, sp.wrapping_add(offset as u32));

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("ADD {{SP}}, ${:04X}", offset);
    // }
}

/// Thumb instruction: Load address
pub fn thumb_load_address(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;

    let destination: u32 = ((instruction >> 8) & 0x7) as u32;
    let offset: u32 = ((instruction & 0x00FF) as u32) << 2;
    let adding_sp: bool = (instruction & (1 << 11)) != 0;

    let address: u32 = if adding_sp {
        emu.get_cpu(cpu_type).get_register(REG_SP as i32)
    } else {
        // Set bit 1 to zero for alignment safety
        emu.get_cpu(cpu_type).get_pc() & !0x2
    };

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("ADD {{{}}}, ${:08X}", destination, address);
    // }

    emu.get_cpu_mut(cpu_type)
        .add(destination, address, offset, false);
}

/// Thumb instruction: PUSH
pub fn thumb_push(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;
    let reg_list: u8 = (instruction & 0x00FF) as u8;

    let mut stack_pointer: u32 = emu.get_cpu(cpu_type).get_register(REG_SP as i32);

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("PUSH ${:02X}", reg_list);
    // }

    let mut regs: i32 = 0;

    if (instruction & (1 << 8)) != 0 {
        regs += 1;
        stack_pointer = stack_pointer.wrapping_sub(4);
        let lr = emu.get_cpu(cpu_type).get_register(REG_LR as i32);
        emu.write_word(stack_pointer, lr, cpu_type);
    }

    for i in (0..8).rev() {
        let bit: u8 = 1 << i;
        if (reg_list & bit) != 0 {
            regs += 1;
            stack_pointer = stack_pointer.wrapping_sub(4);
            let value = emu.get_cpu(cpu_type).get_register(i);
            emu.write_word(stack_pointer, value, cpu_type);
        }
    }

    emu.get_cpu_mut(cpu_type).add_n32_data(stack_pointer, 2);
    if regs > 2 {
        emu.get_cpu_mut(cpu_type)
            .add_s32_data(stack_pointer, regs - 2);
    }

    emu.get_cpu_mut(cpu_type)
        .set_register(REG_SP as i32, stack_pointer);
}

/// Thumb instruction: POP
pub fn thumb_pop(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;
    let reg_list: u8 = (instruction & 0x00FF) as u8;

    let mut stack_pointer: u32 = emu.get_cpu(cpu_type).get_register(REG_SP as i32);

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("POP ${:02X}", reg_list);
    // }

    let mut regs: i32 = 0;

    for i in 0..8 {
        let bit: u8 = 1 << i;
        if (reg_list & bit) != 0 {
            regs += 1;
            let value = emu.read_word(stack_pointer, cpu_type);
            emu.get_cpu_mut(cpu_type).set_register(i, value);
            stack_pointer = stack_pointer.wrapping_add(4);
        }
    }

    if (instruction & (1 << 8)) != 0 {
        // Only ARM9 can change thumb state by popping PC off the stack
        let change_thumb_state: bool = emu.get_cpu(cpu_type).get_id() <= 0;
        let pc_value = emu.read_word(stack_pointer, cpu_type);
        emu.get_cpu_mut(cpu_type).jp(pc_value, change_thumb_state);

        regs += 1;
        stack_pointer = stack_pointer.wrapping_add(4);
    }

    if regs > 1 {
        emu.get_cpu_mut(cpu_type)
            .add_s32_data(stack_pointer, regs - 1);
    }
    emu.get_cpu_mut(cpu_type).add_n32_data(stack_pointer, 1);
    emu.get_cpu_mut(cpu_type).add_internal_cycles(1);

    emu.get_cpu_mut(cpu_type)
        .set_register(REG_SP as i32, stack_pointer);
}

/// Thumb instruction: Store multiple registers
pub fn thumb_store_multiple(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;
    let reg_list: u8 = (instruction & 0x00FF) as u8;
    let base: u32 = ((instruction >> 8) & 0x7) as u32;

    let mut address: u32 = emu.get_cpu(cpu_type).get_register(base as i32);

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("STMIA {{{}}}, ${:02X}", base, reg_list);
    // }

    let mut regs: i32 = 0;

    for reg in 0..8 {
        let bit: u8 = 1 << reg;
        if (reg_list & bit) != 0 {
            regs += 1;
            let value = emu.get_cpu(cpu_type).get_register(reg);
            emu.write_word(address, value, cpu_type);
            address = address.wrapping_add(4);
        }
    }

    emu.get_cpu_mut(cpu_type).add_n32_data(address, 2);
    if regs > 2 {
        emu.get_cpu_mut(cpu_type).add_s32_data(address, regs - 2);
    }

    emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
}

/// Thumb instruction: Load multiple registers
fn thumb_load_multiple(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;
    let reg_list: u8 = (instruction & 0x00FF) as u8;
    let base: u32 = ((instruction >> 8) & 0x7) as u32;

    let mut address: u32 = emu.get_cpu(cpu_type).get_register(base as i32);

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("LDMIA {{{}}}, ${:02X}", base, reg_list);
    // }

    let mut regs: i32 = 0;

    for reg in 0..8 {
        let bit: u8 = 1 << reg;
        if (reg_list & bit) != 0 {
            regs += 1;
            let value = emu.read_word(address, cpu_type);
            emu.get_cpu_mut(cpu_type).set_register(reg, value);
            address = address.wrapping_add(4);
        }
    }

    emu.get_cpu_mut(cpu_type).add_n32_data(address, 2);
    if regs > 1 {
        emu.get_cpu_mut(cpu_type).add_s32_data(address, regs - 2);
    }
    emu.get_cpu_mut(cpu_type).add_internal_cycles(1);

    let base_bit: u8 = 1 << base;
    if (reg_list & base_bit) == 0 {
        emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
    }
}

/// Thumb instruction: Branch
pub fn thumb_branch(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;
    let mut address: u32 = emu.get_cpu(cpu_type).get_pc();

    // int16_t offset = (instruction & 0x7FF) << 1;
    let mut offset: i16 = ((instruction & 0x07FF) << 1) as i16;

    // Sign extend 12-bit offset
    offset = (offset << 4) >> 4;

    address = address.wrapping_add(offset as u32);

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("B ${:04X}", address);
    // }

    emu.get_cpu_mut(cpu_type).jp(address, false);
}

/// Thumb instruction: Conditional branch
fn thumb_cond_branch(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;
    let condition: u32 = ((instruction >> 8) & 0xF) as u32;

    if condition == 0xF {
        // if emu.get_cpu_mut(cpu_type).can_disassemble() {
        //     println!("SWI ${:02X}", instruction & 0xFF);
        // }
        emu.get_cpu_mut(cpu_type).handle_swi();
        return;
    }

    let address: u32 = emu.get_cpu(cpu_type).get_pc();

    // int16_t offset = static_cast<int32_t>(instruction << 24) >> 23;
    let offset: i32 = ((instruction as i32) << 24) >> 23;

    /* if emu.get_cpu_mut(cpu_type).can_disassemble() {
        print!("B");
        emu.get_cpu_mut(cpu_type).print_condition(condition);
        println!(" ${:08X}", address.wrapping_add(offset as u32));
    } */

    if emu.get_cpu(cpu_type).check_condition(condition as i32) {
        emu.get_cpu_mut(cpu_type)
            .jp(address.wrapping_add(offset as u32), false);
    }
}

/// Thumb instruction: Prepare long branch
#[allow(clippy::missing_const_for_fn)]
pub fn thumb_long_branch_prep(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;
    let mut upper_address: u32 = emu.get_cpu(cpu_type).get_pc();

    // int32_t offset = ((instruction & 0x7FF) << 21) >> 9;
    let offset: i32 = (((instruction & 0x07FF) as i32) << 21) >> 9;
    upper_address = upper_address.wrapping_add(offset as u32);

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("BLP: ${:08X}", upper_address);
    // }

    emu.get_cpu_mut(cpu_type)
        .set_register(REG_LR as i32, upper_address);
}

/// Thumb instruction: Long branch
pub fn thumb_long_branch(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu(cpu_type).get_current_instr() as u16;
    let mut address: u32 = emu.get_cpu(cpu_type).get_register(REG_LR as i32);

    // address += (instruction & 0x7FF) << 1;
    address = address.wrapping_add(((instruction & 0x07FF) as u32) << 1);

    let mut new_lr: u32 = emu.get_cpu(cpu_type).get_pc().wrapping_sub(2);
    new_lr |= 0x1; // Preserve Thumb mode

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("BL: ${:08X}", address);
    // }

    emu.get_cpu_mut(cpu_type)
        .set_register(REG_LR as i32, new_lr);
    emu.get_cpu_mut(cpu_type).jp(address, false);
}

/// Thumb instruction: Long branch with link and exchange
pub fn thumb_long_blx(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u16 = emu.get_cpu_mut(cpu_type).get_current_instr() as u16;
    let mut address: u32 = emu.get_cpu_mut(cpu_type).get_register(REG_LR as i32);
    address += (instruction & 0x7FF) as u32 * 2; // << 1

    let mut new_lr = emu.get_cpu(cpu_type).get_pc() - 2;
    new_lr |= 0x1; // Preserve Thumb mode

    // if emu.get_cpu_mut(cpu_type).can_disassemble() {
    //     println!("BLX: {}", address);
    // }

    // Set LR to return address
    emu.get_cpu_mut(cpu_type)
        .set_register(REG_LR as i32, new_lr);

    // Switch to ARM mode
    emu.get_cpu_mut(cpu_type).jp(address, true);
}
