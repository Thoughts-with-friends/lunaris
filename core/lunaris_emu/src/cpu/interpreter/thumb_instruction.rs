// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! instrthumb.cpp
//!

use crate::cpu::arm_cpu::{ArmCpu, REG_LR, REG_PC, REG_SP};
use crate::cpu::instruction_table::ThumbInstr;

/// Interprets a Thumb instruction
pub fn thumb_interpret(cpu: &mut ArmCpu) {
    // NOTE: tracing output is gated behind the `tracing` feature
    let instruction = cpu.get_current_instr() & 0xFFFF;
    let cpu_id = cpu.get_id();

    if cpu_id > 0 {
        #[cfg(feature = "tracing")]
        {
            if !cpu_id > 0 {
                tracing::error!("(9T)");
            } else {
                tracing::error!("(7T)");
            }

            tracing::error!(" ${:08X} - ", cpu.get_pc().wrapping_sub(4));
            tracing::error!("(${:#06X}) ", instruction);
        }
    }

    let opcode = thumb_decode(instruction);

    match opcode {
        ThumbInstr::MovShift => thumb_mov_shift(cpu),
        ThumbInstr::AddReg => thumb_add_reg(cpu),
        ThumbInstr::SubReg => thumb_sub_reg(cpu),
        ThumbInstr::MovImm => thumb_mov(cpu),
        ThumbInstr::CmpImm => thumb_cmp(cpu),
        ThumbInstr::AddImm => thumb_add(cpu),
        ThumbInstr::SubImm => thumb_sub(cpu),
        ThumbInstr::AluOp => thumb_alu_op(cpu),
        ThumbInstr::HiRegOp => thumb_hi_reg_op(cpu),
        ThumbInstr::PcRelLoad => thumb_pc_rel_load(cpu),
        ThumbInstr::StoreImmOffset => thumb_store_imm_offset(cpu),
        ThumbInstr::LoadImmOffset => thumb_load_imm_offset(cpu),
        ThumbInstr::StoreRegOffset => thumb_store_reg_offset(cpu),
        ThumbInstr::LoadRegOffset => thumb_load_reg_offset(cpu),
        ThumbInstr::StoreHalfword => thumb_store_halfword(cpu),
        ThumbInstr::LoadHalfword => thumb_load_halfword(cpu),
        ThumbInstr::LoadStoreSignHalfword => thumb_load_store_sign_halfword(cpu),
        ThumbInstr::SpRelStore => thumb_sp_rel_store(cpu),
        ThumbInstr::SpRelLoad => thumb_sp_rel_load(cpu),
        ThumbInstr::OffsetSp => thumb_offset_sp(cpu),
        ThumbInstr::LoadAddress => thumb_load_address(cpu),
        ThumbInstr::LoadMultiple => thumb_load_multiple(cpu),
        ThumbInstr::StoreMultiple => thumb_store_multiple(cpu),
        ThumbInstr::Push => thumb_push(cpu),
        ThumbInstr::Pop => thumb_pop(cpu),
        ThumbInstr::Branch => thumb_branch(cpu),
        ThumbInstr::CondBranch => thumb_cond_branch(cpu),
        ThumbInstr::LongBranchPrep => thumb_long_branch_prep(cpu),
        ThumbInstr::LongBranch => thumb_long_branch(cpu),
        ThumbInstr::LongBlx => thumb_long_blx(cpu),

        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!(
                "Unrecognized Thumb opcode ${:04X}",
                cpu.get_current_instr() & 0xFFFF
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
pub fn thumb_mov_shift(cpu: &mut ArmCpu) {
    let instruction = cpu.get_current_instr() as u16;
    let opcode = ((instruction >> 11) & 0x3) as u32;
    let mut shift = ((instruction >> 6) & 0x1F) as u32;
    let source = ((instruction >> 3) & 0x7) as u32;
    let destination = (instruction & 0x7) as u32;

    let mut value: u32 = cpu.get_register(source as i32);

    match opcode {
        0 => {
            value = cpu.lsl(value, shift as i32, true);
            // if cpu.get_id() > 0 {
            //     // println!("LSL {{{}}}, {{{}}}, #{}", destination, source, shift);
            // }
        }
        1 => {
            if shift == 0 {
                shift = 32;
            }
            value = cpu.lsr(value, shift as i32, true);
            // if cpu.get_id() > 0 {
            //     // println!("LSR {{{}}}, {{{}}}, #{}", destination, source, shift);
            // }
        }
        2 => {
            if shift == 0 {
                shift = 32;
            }
            value = cpu.asr(value, shift as i32, true);
            // if cpu.get_id() > 0 {
            //     // println!("ASR {{{}}}, {{{}}}, #{}", destination, source, shift);
            // }
        }
        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!("Unrecognized opcode {opcode} in thumb_mov_shift");
        }
    }

    cpu.add_internal_cycles(1); // Extra cycle due to register shift
    cpu.set_register(destination as i32, value);
    // Optionally: cpu.set_lo_register(destination, value);
}

/// Thumb instruction: ADD register
pub fn thumb_add_reg(cpu: &mut ArmCpu) {
    let instruction = cpu.get_current_instr() as u16;
    let destination = (instruction & 0x7) as u32;
    let source = ((instruction >> 3) & 0x7) as u32;
    let mut operand = ((instruction >> 6) & 0x7) as u32;
    let is_imm = (instruction & (1 << 10)) != 0;

    if !is_imm {
        // if cpu.get_id() > 0 {
        //     // println!("ADD {{{}}}, {{{}}}, {{{}}}", destination, source, operand);
        // }
        operand = cpu.get_register(operand as i32);
    }
    // else if cpu.get_id() > 0 {
    //     // println!("ADD {{{}}}, {{{}}}, ${:08X}", destination, source, operand);
    // }
    let src = cpu.get_register(source as i32);
    cpu.add(destination, src, operand, true);
}

/// Thumb instruction: SUB register
pub fn thumb_sub_reg(cpu: &mut ArmCpu) {
    let instruction = cpu.get_current_instr() as u16;
    let destination = (instruction & 0x7) as u32;
    let source = ((instruction >> 3) & 0x7) as u32;
    let mut operand = ((instruction >> 6) & 0x7) as u32;
    let is_imm = (instruction & (1 << 10)) != 0;

    if !is_imm {
        if cpu.get_id() > 0 {
            // println!("SUB {{{}}}, {{{}}}, {{{}}}", destination, source, operand);
        }
        operand = cpu.get_register(operand as i32);
    } else if cpu.get_id() > 0 {
        // println!("SUB {{{}}}, {{{}}}, ${:08X}", destination, source, operand);
    }

    cpu.sub(destination, cpu.get_register(source as i32), operand, true);
}

/// Thumb instruction: MOV immediate
pub fn thumb_mov(cpu: &mut ArmCpu) {
    let instruction = cpu.get_current_instr() as u16;
    let offset = (instruction & 0xFF) as u32;
    let reg = ((instruction >> 8) & 0x7) as u32;

    // if cpu.get_id() > 0 {
    //     // println!("MOV {{{}}}, ${:02X}", reg, offset);
    // }

    cpu.mov(reg, offset, true);
}

/// Thumb instruction: CMP
pub const fn thumb_cmp(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let offset: u32 = (instruction & 0xFF) as u32;
    let reg: u32 = ((instruction >> 8) & 0x7) as u32;

    // if cpu.get_id() > 0 {
    //     // println!("CMP {{{}}}, ${:02X}", reg, offset);
    // }

    cpu.cmp(cpu.get_register(reg as i32), offset);
}

/// Thumb instruction: ADD
pub fn thumb_add(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let offset: u32 = (instruction & 0xFF) as u32;
    let reg: u32 = ((instruction >> 8) & 0x7) as u32;

    // if cpu.get_id() > 0 {
    //     // println!("ADD {{{}}}, ${:02X}", reg, offset);
    // }

    cpu.add(reg, cpu.get_register(reg as i32), offset, true);
}

/// Thumb instruction: SUB
pub fn thumb_sub(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let offset: u32 = (instruction & 0xFF) as u32;
    let reg: u32 = ((instruction >> 8) & 0x7) as u32;

    // if cpu.get_id() > 0 {
    //     // println!("SUB {{{}}}, ${:02X}", reg, offset);
    // }

    cpu.sub(reg, cpu.get_register(reg as i32), offset, true);
}

/// Thumb instruction: ALU operations
#[allow(clippy::missing_const_for_fn)]
pub fn thumb_alu_op(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let destination = (instruction & 0x7) as u32;
    let source = ((instruction >> 3) & 0x7) as u32;
    let opcode = ((instruction >> 6) & 0xF) as u32;

    match opcode {
        0x0 => {
            // AND
            // if cpu.get_id() > 0 {
            //     // println!("AND {{{}}}, {{{}}}", destination, source);
            // }
            let src = cpu.get_register(destination as i32) as i32;
            let operand = cpu.get_register(source as i32) as i32;
            cpu.andd(destination as i32, src, operand, true);
        }
        0x1 => {
            // EOR
            // if cpu.get_id() > 0 {
            //     // println!("EOR {{{}}}, {{{}}}", destination, source);
            // }
            let src = cpu.get_register(destination as i32) as i32;
            let operand = cpu.get_register(source as i32) as i32;
            cpu.eor(destination as i32, src, operand, true);
        }
        0x2 => {
            // LSL
            // if cpu.get_id() > 0 {
            //     // println!("LSL {{{}}}, {{{}}}", destination, source);
            // }
            let mut reg = cpu.get_register(destination as i32);
            let shift = cpu.get_register(source as i32) as i32;
            reg = cpu.lsl(reg, shift, true);
            cpu.set_register(destination as i32, reg);
        }
        0x3 => {
            // LSR
            // if cpu.get_id() > 0 {
            //     // println!("LSR {{{}}}, {{{}}}", destination, source);
            // }
            let mut reg = cpu.get_register(destination as i32);
            let shift = cpu.get_register(source as i32) as i32;
            reg = cpu.lsr(reg, shift, true);
            cpu.set_register(destination as i32, reg);
        }
        0x4 => {
            // ASR
            // if cpu.get_id() > 0 {
            //     // println!("ASR {{{}}}, {{{}}}", destination, source);
            // }
            let mut reg = cpu.get_register(destination as i32);
            let shift = cpu.get_register(source as i32) as i32;
            reg = cpu.asr(reg, shift, true);
            cpu.set_register(destination as i32, reg);
        }
        0x5 => {
            // ADC
            // if cpu.get_id() > 0 {
            //     // println!("ADC {{{}}}, {{{}}}", destination, source);
            // }
            cpu.adc(
                destination,
                cpu.get_register(destination as i32),
                cpu.get_register(source as i32),
                true,
            );
        }
        0x6 => {
            // SBC
            // if cpu.get_id() > 0 {
            //     // println!("SBC {{{}}}, {{{}}}", destination, source);
            // }
            cpu.sbc(
                destination,
                cpu.get_register(destination as i32),
                cpu.get_register(source as i32),
                true,
            );
        }
        0x7 => {
            // ROR
            if cpu.get_id() > 0 {
                // println!("ROR {{{}}}, {{{}}}", destination, source);
            }
            let mut reg = cpu.get_register(destination as i32);
            reg = cpu.rotr32(reg, cpu.get_register(source as i32), true);
            cpu.set_register(destination as i32, reg);
        }
        0x8 => {
            // TST
            // if cpu.get_id() > 0 {
            //     // println!("TST {{{}}}, {{{}}}", destination, source);
            // }
            cpu.tst(
                cpu.get_register(destination as i32),
                cpu.get_register(source as i32),
            );
        }
        0x9 => {
            // NEG
            // if cpu.get_id() > 0 {
            //     // println!("NEG {{{}}}, {{{}}}", destination, source);
            // }
            cpu.sub(destination, 0, cpu.get_register(source as i32), true);
        }
        0xA => {
            // CMP
            // if cpu.get_id() > 0 {
            //     // println!("CMP {{{}}}, {{{}}}", destination, source);
            // }
            cpu.cmp(
                cpu.get_register(destination as i32),
                cpu.get_register(source as i32),
            );
        }
        0xB => {
            // CMN
            if cpu.get_id() > 0 {
                // println!("CMN {{{}}}, {{{}}}", destination, source);
            }
            cpu.cmn(
                cpu.get_register(destination as i32),
                cpu.get_register(source as i32),
            );
        }
        0xC => {
            // ORR
            if cpu.get_id() > 0 {
                // println!("ORR {{{}}}, {{{}}}", destination, source);
            }
            let reg = cpu.get_register(destination as i32) as i32;
            let src = cpu.get_register(source as i32) as i32;
            cpu.orr(destination as i32, reg, src, true);
        }
        0xD => {
            // MUL
            // if cpu.get_id() > 0 {
            //     // println!("MUL {{{}}}, {{{}}}", destination, source);
            // }
            cpu.mul(
                destination,
                cpu.get_register(destination as i32),
                cpu.get_register(source as i32),
                true,
            );
            if !cpu.get_id() > 0 {
                cpu.add_internal_cycles(3);
            } else {
                let multiplicand = cpu.get_register(destination as i32);
                if multiplicand & 0xFF000000 != 0 {
                    cpu.add_internal_cycles(4);
                } else if multiplicand & 0x00FF0000 != 0 {
                    cpu.add_internal_cycles(3);
                } else if multiplicand & 0x0000FF00 != 0 {
                    cpu.add_internal_cycles(2);
                } else {
                    cpu.add_internal_cycles(1);
                }
            }
        }
        0xE => {
            // BIC
            if cpu.get_id() > 0 {
                // println!("BIC {{{}}}, {{{}}}", destination, source);
            }
            cpu.bic(
                destination,
                cpu.get_register(destination as i32),
                cpu.get_register(source as i32),
                true,
            );
        }
        0xF => {
            // MVN
            if cpu.get_id() > 0 {
                // println!("MVN {{{}}}, {{{}}}", destination, source);
            }
            cpu.mvn(destination, cpu.get_register(source as i32), true);
        }
        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!("Invalid thumb alu op {opcode}");
        }
    }
}

/// Thumb instruction: High register operations
#[allow(clippy::missing_const_for_fn)]
pub fn thumb_hi_reg_op(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

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
            // if cpu.get_id() > 0 {
            //     // println!("ADD {{{}}}, {{{}}}", destination, source);
            // }
            if destination == REG_PC {
                cpu.jp(
                    cpu.get_pc().wrapping_add(cpu.get_register(source as i32)),
                    false,
                );
            } else {
                cpu.add(
                    destination,
                    cpu.get_register(destination as i32),
                    cpu.get_register(source as i32),
                    false,
                );
            }
        }
        0x1 => {
            // CMP
            if cpu.get_id() > 0 {
                // println!("CMP {{{}}}, {{{}}}", destination, source);
            }
            cpu.cmp(
                cpu.get_register(destination as i32),
                cpu.get_register(source as i32),
            );
        }
        0x2 => {
            // MOV
            if cpu.get_id() > 0 {
                // println!("MOV {{{}}}, {{{}}}", destination, source);
            }
            if destination == REG_PC {
                cpu.jp(cpu.get_register(source as i32), false);
            } else {
                cpu.mov(destination, cpu.get_register(source as i32), false);
            }
        }
        0x3 => {
            // BX / BLX
            if high_dest {
                // if cpu.get_id() > 0 {
                //     // println!("BLX {{{}}}", source);
                // }
                cpu.set_register(REG_LR as i32, cpu.get_pc().wrapping_sub(1));
            } else if cpu.get_id() > 0 {
                #[cfg(feature = "tracing")]
                tracing::error!("BX {{{source}}}");
            }
            cpu.jp(cpu.get_register(source as i32), true);
        }
        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!("High-reg Thumb opcode ${opcode:02X} not recognized");
        }
    }
}

/// Thumb instruction: PC-relative load
pub fn thumb_pc_rel_load(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let destination: u32 = ((instruction >> 8) & 0x7) as u32;
    let mut address: u32 = cpu
        .get_pc()
        .wrapping_add(((instruction & 0xFF) as u32) << 2);
    address &= !0x3; // 4-byte alignment

    // if cpu.get_id() > 0 {
    //     // println!("LDR {{{}}}, ${:08X}", destination, address);
    // }

    cpu.add_n32_data(address, 1);
    cpu.add_internal_cycles(1);
    let value = cpu.read_word(address);
    cpu.set_register(destination as i32, value);
}

/// Thumb instruction: Store register with offset
pub fn thumb_store_reg_offset(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let is_byte: bool = (instruction & (1 << 10)) != 0;
    let base: u32 = ((instruction >> 3) & 0x7) as u32;
    let source: u32 = (instruction & 0x7) as u32;
    let offset: u32 = ((instruction >> 6) & 0x7) as u32;

    let mut address: u32 = cpu.get_register(base as i32);
    address = address.wrapping_add(cpu.get_register(offset as i32));

    let source_contents: u32 = cpu.get_register(source as i32);

    if is_byte {
        // if cpu.get_id() > 0 {
        //     // println!("STRB {{{}}}, [{{{}}}, {{{}}}]", source, base, offset);
        // }
        cpu.add_n16_data(address, 1);
        cpu.write_byte(address, (source_contents & 0xFF) as u8);
    } else {
        if cpu.get_id() > 0 {
            // println!("STR {{{}}}, [{{{}}}, {{{}}}]", source, base, offset);
        }
        cpu.add_n32_data(address, 1);
        cpu.write_word(address, source_contents);
    }
}

/// Thumb instruction: Load register with offset
pub fn thumb_load_reg_offset(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let is_byte: bool = (instruction & (1 << 10)) != 0;
    let base = ((instruction >> 3) & 0x7) as u32;
    let destination = (instruction & 0x7) as u32;
    let offset = ((instruction >> 6) & 0x7) as u32;

    let mut address: u32 = cpu.get_register(base as i32);
    address = address.wrapping_add(cpu.get_register(offset as i32));

    if is_byte {
        // if cpu.get_id() > 0 {
        //     // println!("LDRB {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
        // }
        cpu.add_n16_data(address, 1);
        cpu.add_internal_cycles(1);
        let value = cpu.read_byte(address);
        cpu.set_register(destination as i32, value as u32);
    } else {
        if cpu.get_id() > 0 {
            // println!("LDR {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
        }
        cpu.add_n32_data(address, 1);
        cpu.add_internal_cycles(1);
        let aligned_addr = address & !0x3;
        let rotate = (address & 0x3) * 8;
        let addr = cpu.read_word(aligned_addr);
        let word = cpu.rotr32(addr, rotate, false);
        cpu.set_register(destination as i32, word);
    }
}

/// Thumb instruction: Load halfword
pub fn thumb_load_halfword(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let offset: u32 = (((instruction >> 6) & 0x1F) << 1) as u32;
    let base: u32 = ((instruction >> 3) & 0x7) as u32;
    let source: u32 = (instruction & 0x7) as u32;

    let address: u32 = cpu.get_register(base as i32).wrapping_add(offset);
    let value = (cpu.get_register(source as i32) & 0xFFFF) as u16;

    if cpu.get_id() > 0 {
        // println!("STRH {{{}}}, [{{{}}}, ${:04X}]", source, base, offset);
    }

    cpu.add_n16_data(address, 1);
    cpu.write_halfword(address, value);
}

/// Thumb instruction: Store halfword
pub fn thumb_store_halfword(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let offset: u32 = (((instruction >> 6) & 0x1F) << 1) as u32;
    let base = ((instruction >> 3) & 0x7) as u32;
    let destination = (instruction & 0x7) as u32;

    let address: u32 = cpu.get_register(base as i32).wrapping_add(offset);

    if cpu.get_id() > 0 {
        // println!("LDRH {{{}}}, [{{{}}}, ${:04X}]", destination, base, offset);
    }

    cpu.add_n16_data(address, 1);
    let value = cpu.read_halfword(address);
    cpu.set_register(destination as i32, value as u32);
}

/// Thumb instruction: Store with immediate offset
pub fn thumb_store_imm_offset(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let source: u32 = (instruction & 0x7) as u32;
    let base: u32 = ((instruction >> 3) & 0x7) as u32;
    let mut offset: u32 = ((instruction >> 6) & 0x1F) as u32;
    let is_byte: bool = (instruction & (1 << 12)) != 0;

    let mut address: u32 = cpu.get_register(base as i32);

    if is_byte {
        address = address.wrapping_add(offset);

        if cpu.get_id() > 0 {
            // println!("STRB {{{}}}, [{{{}}}, ${:02X}]", source, base, offset);
        }

        cpu.add_n16_data(address, 1);
        let value = (cpu.get_register(source as i32) & 0xFF) as u8;
        cpu.write_byte(address, value);
    } else {
        offset <<= 2;
        address = address.wrapping_add(offset);

        if cpu.get_id() > 0 {
            // println!("STR {{{}}}, [{{{}}}, ${:02X}]", source, base, offset);
        }

        cpu.add_n32_data(address, 1);
        let value = cpu.get_register(source as i32);
        cpu.write_word(address, value);
    }
}

/// Thumb instruction: Load with immediate offset
pub fn thumb_load_imm_offset(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let destination: u32 = (instruction & 0x7) as u32;
    let base: u32 = ((instruction >> 3) & 0x7) as u32;
    let mut offset: u32 = ((instruction >> 6) & 0x1F) as u32;
    let is_byte: bool = (instruction & (1 << 12)) != 0;

    let mut address: u32 = cpu.get_register(base as i32);

    if is_byte {
        address = address.wrapping_add(offset);

        if cpu.get_id() > 0 {
            // println!("LDRB {{{}}}, [{{{}}}, ${:02X}]", destination, base, offset);
        }

        cpu.add_n16_data(address, 1);
        let value = cpu.read_byte(address);
        cpu.set_register(destination as i32, value as u32);
    } else {
        offset <<= 2;
        address = address.wrapping_add(offset);

        if cpu.get_id() > 0 {
            // println!("LDR {{{}}}, [{{{}}}, ${:02X}]", destination, base, offset);
        }

        cpu.add_n32_data(address, 1);

        let aligned_addr = address & !0x3;
        let rotate = (address & 0x3) * 8;
        let addr = cpu.read_word(aligned_addr);
        let word = cpu.rotr32(addr, rotate, false);

        cpu.set_register(destination as i32, word);
    }

    cpu.add_internal_cycles(1);
}

/// Thumb instruction: Load/store signed halfword
pub fn thumb_load_store_sign_halfword(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let destination: u32 = (instruction & 0x7) as u32;
    let base: u32 = ((instruction >> 3) & 0x7) as u32;
    let offset: u32 = ((instruction >> 6) & 0x7) as u32;
    let opcode: u32 = ((instruction >> 10) & 0x3) as u32;

    let mut address: u32 = cpu.get_register(base as i32);
    address = address.wrapping_add(cpu.get_register(offset as i32));

    match opcode {
        0 => {
            // STRH
            if cpu.get_id() > 0 {
                // println!("STRH {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
            }
            let value = (cpu.get_register(destination as i32) & 0xFFFF) as u16;
            cpu.write_halfword(address, value);
            cpu.add_n32_data(address, 1);
        }

        1 => {
            // LDSB
            if cpu.get_id() > 0 {
                // println!("LDSB {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
            }
            let mut extended_byte: u32 = cpu.read_byte(address).into();
            extended_byte = (extended_byte as i8 as i32) as u32;
            cpu.set_register(destination as i32, extended_byte);
            cpu.add_internal_cycles(1);
            cpu.add_n16_data(address, 1);
        }

        2 => {
            // LDRH
            if cpu.get_id() > 0 {
                // println!("LDRH {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
            }
            let value = cpu.read_halfword(address);
            cpu.set_register(destination as i32, value as u32);
            cpu.add_n16_data(address, 1);
            cpu.add_internal_cycles(1);
        }

        3 => {
            // LDSH
            if cpu.get_id() > 0 {
                // println!("LDSH {{{}}}, [{{{}}}, {{{}}}]", destination, base, offset);
            }
            let mut extended_halfword: u32 = cpu.read_halfword(address).into();
            extended_halfword = (extended_halfword as i16 as i32) as u32;
            cpu.set_register(destination as i32, extended_halfword);
            cpu.add_internal_cycles(1);
            cpu.add_n16_data(address, 1);
        }

        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!("Sign extended opcode {opcode} not recognized");
        }
    }
}

/// Thumb instruction: Stack pointer-relative store
pub fn thumb_sp_rel_store(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let source: u32 = ((instruction >> 8) & 0x7) as u32;
    let offset: u32 = ((instruction & 0x00FF) as u32) << 2;

    let mut address: u32 = cpu.get_register(REG_SP as i32);
    address = address.wrapping_add(offset);

    // if cpu.can_disassemble() {
    //     println!("STR {{{}}}, [SP, ${:04X}]", source, offset);
    // }

    cpu.add_n32_data(address, 1);
    let value = cpu.get_register(source as i32);
    cpu.write_word(address, value);
}

/// Thumb instruction: Stack pointer-relative load
pub fn thumb_sp_rel_load(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let destination: u32 = ((instruction >> 8) & 0x7) as u32;
    let offset: u32 = ((instruction & 0x00FF) as u32) << 2;

    let mut address: u32 = cpu.get_register(REG_SP as i32);
    address = address.wrapping_add(offset);

    // if cpu.can_disassemble() {
    //     println!("LDR {{{}}}, [SP, ${:04X}]", destination, offset);
    // }

    cpu.add_n32_data(address, 1);
    cpu.add_internal_cycles(1);

    let value = cpu.read_word(address);
    cpu.set_register(destination as i32, value);
}

/// Thumb instruction: Offset SP operation
pub const fn thumb_offset_sp(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let mut offset: i16 = ((instruction & 0x007F) << 2) as i16;

    if (instruction & (1 << 7)) != 0 {
        offset = -offset;
    }

    let sp: u32 = cpu.get_register(REG_SP as i32);
    cpu.set_register(REG_SP as i32, sp.wrapping_add(offset as u32));

    // if cpu.can_disassemble() {
    //     println!("ADD {{SP}}, ${:04X}", offset);
    // }
}

/// Thumb instruction: Load address
pub fn thumb_load_address(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;

    let destination: u32 = ((instruction >> 8) & 0x7) as u32;
    let offset: u32 = ((instruction & 0x00FF) as u32) << 2;
    let adding_sp: bool = (instruction & (1 << 11)) != 0;

    let address: u32 = if adding_sp {
        cpu.get_register(REG_SP as i32)
    } else {
        // Set bit 1 to zero for alignment safety
        cpu.get_pc() & !0x2
    };

    // if cpu.can_disassemble() {
    //     println!("ADD {{{}}}, ${:08X}", destination, address);
    // }

    cpu.add(destination, address, offset, false);
}

/// Thumb instruction: PUSH
pub fn thumb_push(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let reg_list: u8 = (instruction & 0x00FF) as u8;

    let mut stack_pointer: u32 = cpu.get_register(REG_SP as i32);

    // if cpu.can_disassemble() {
    //     println!("PUSH ${:02X}", reg_list);
    // }

    let mut regs: i32 = 0;

    if (instruction & (1 << 8)) != 0 {
        regs += 1;
        stack_pointer = stack_pointer.wrapping_sub(4);
        let lr = cpu.get_register(REG_LR as i32);
        cpu.write_word(stack_pointer, lr);
    }

    for i in (0..8).rev() {
        let bit: u8 = 1 << i;
        if (reg_list & bit) != 0 {
            regs += 1;
            stack_pointer = stack_pointer.wrapping_sub(4);
            let value = cpu.get_register(i);
            cpu.write_word(stack_pointer, value);
        }
    }

    cpu.add_n32_data(stack_pointer, 2);
    if regs > 2 {
        cpu.add_s32_data(stack_pointer, regs - 2);
    }

    cpu.set_register(REG_SP as i32, stack_pointer);
}

/// Thumb instruction: POP
pub fn thumb_pop(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let reg_list: u8 = (instruction & 0x00FF) as u8;

    let mut stack_pointer: u32 = cpu.get_register(REG_SP as i32);

    // if cpu.can_disassemble() {
    //     println!("POP ${:02X}", reg_list);
    // }

    let mut regs: i32 = 0;

    for i in 0..8 {
        let bit: u8 = 1 << i;
        if (reg_list & bit) != 0 {
            regs += 1;
            let value = cpu.read_word(stack_pointer);
            cpu.set_register(i, value);
            stack_pointer = stack_pointer.wrapping_add(4);
        }
    }

    if (instruction & (1 << 8)) != 0 {
        // Only ARM9 can change thumb state by popping PC off the stack
        let change_thumb_state: bool = cpu.get_id() <= 0;
        let pc_value = cpu.read_word(stack_pointer);
        cpu.jp(pc_value, change_thumb_state);

        regs += 1;
        stack_pointer = stack_pointer.wrapping_add(4);
    }

    if regs > 1 {
        cpu.add_s32_data(stack_pointer, regs - 1);
    }
    cpu.add_n32_data(stack_pointer, 1);
    cpu.add_internal_cycles(1);

    cpu.set_register(REG_SP as i32, stack_pointer);
}

/// Thumb instruction: Store multiple registers
pub fn thumb_store_multiple(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let reg_list: u8 = (instruction & 0x00FF) as u8;
    let base: u32 = ((instruction >> 8) & 0x7) as u32;

    let mut address: u32 = cpu.get_register(base as i32);

    // if cpu.can_disassemble() {
    //     println!("STMIA {{{}}}, ${:02X}", base, reg_list);
    // }

    let mut regs: i32 = 0;

    for reg in 0..8 {
        let bit: u8 = 1 << reg;
        if (reg_list & bit) != 0 {
            regs += 1;
            let value = cpu.get_register(reg);
            cpu.write_word(address, value);
            address = address.wrapping_add(4);
        }
    }

    cpu.add_n32_data(address, 2);
    if regs > 2 {
        cpu.add_s32_data(address, regs - 2);
    }

    cpu.set_register(base as i32, address);
}

/// Thumb instruction: Load multiple registers
fn thumb_load_multiple(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let reg_list: u8 = (instruction & 0x00FF) as u8;
    let base: u32 = ((instruction >> 8) & 0x7) as u32;

    let mut address: u32 = cpu.get_register(base as i32);

    // if cpu.can_disassemble() {
    //     println!("LDMIA {{{}}}, ${:02X}", base, reg_list);
    // }

    let mut regs: i32 = 0;

    for reg in 0..8 {
        let bit: u8 = 1 << reg;
        if (reg_list & bit) != 0 {
            regs += 1;
            let value = cpu.read_word(address);
            cpu.set_register(reg, value);
            address = address.wrapping_add(4);
        }
    }

    cpu.add_n32_data(address, 2);
    if regs > 1 {
        cpu.add_s32_data(address, regs - 2);
    }
    cpu.add_internal_cycles(1);

    let base_bit: u8 = 1 << base;
    if (reg_list & base_bit) == 0 {
        cpu.set_register(base as i32, address);
    }
}

/// Thumb instruction: Branch
pub const fn thumb_branch(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let mut address: u32 = cpu.get_pc();

    // int16_t offset = (instruction & 0x7FF) << 1;
    let mut offset: i16 = ((instruction & 0x07FF) << 1) as i16;

    // Sign extend 12-bit offset
    offset = (offset << 4) >> 4;

    address = address.wrapping_add(offset as u32);

    // if cpu.can_disassemble() {
    //     println!("B ${:04X}", address);
    // }

    cpu.jp(address, false);
}

/// Thumb instruction: Conditional branch
fn thumb_cond_branch(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let condition: u32 = ((instruction >> 8) & 0xF) as u32;

    if condition == 0xF {
        // if cpu.can_disassemble() {
        //     println!("SWI ${:02X}", instruction & 0xFF);
        // }
        cpu.handle_swi();
        return;
    }

    let address: u32 = cpu.get_pc();

    // int16_t offset = static_cast<int32_t>(instruction << 24) >> 23;
    let offset: i32 = ((instruction as i32) << 24) >> 23;

    /* if cpu.can_disassemble() {
        print!("B");
        cpu.print_condition(condition);
        println!(" ${:08X}", address.wrapping_add(offset as u32));
    } */

    if cpu.check_condition(condition as i32) {
        cpu.jp(address.wrapping_add(offset as u32), false);
    }
}

/// Thumb instruction: Prepare long branch
#[allow(clippy::missing_const_for_fn)]
pub fn thumb_long_branch_prep(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let mut upper_address: u32 = cpu.get_pc();

    // int32_t offset = ((instruction & 0x7FF) << 21) >> 9;
    let offset: i32 = (((instruction & 0x07FF) as i32) << 21) >> 9;
    upper_address = upper_address.wrapping_add(offset as u32);

    // if cpu.can_disassemble() {
    //     println!("BLP: ${:08X}", upper_address);
    // }

    cpu.set_register(REG_LR as i32, upper_address);
}

/// Thumb instruction: Long branch
pub const fn thumb_long_branch(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let mut address: u32 = cpu.get_register(REG_LR as i32);

    // address += (instruction & 0x7FF) << 1;
    address = address.wrapping_add(((instruction & 0x07FF) as u32) << 1);

    let mut new_lr: u32 = cpu.get_pc().wrapping_sub(2);
    new_lr |= 0x1; // Preserve Thumb mode

    // if cpu.can_disassemble() {
    //     println!("BL: ${:08X}", address);
    // }

    cpu.set_register(REG_LR as i32, new_lr);
    cpu.jp(address, false);
}

/// Thumb instruction: Long branch with link and exchange
pub const fn thumb_long_blx(cpu: &mut ArmCpu) {
    let instruction: u16 = cpu.get_current_instr() as u16;
    let mut address: u32 = cpu.get_register(REG_LR as i32);
    address += (instruction & 0x7FF) as u32 * 2; // << 1

    let mut new_lr = cpu.get_pc() - 2;
    new_lr |= 0x1; // Preserve Thumb mode

    // if cpu.can_disassemble() {
    //     println!("BLX: {}", address);
    // }

    // Set LR to return address
    cpu.set_register(REG_LR as i32, new_lr);

    // Switch to ARM mode
    cpu.jp(address, true);
}
