// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! cpuinsters.hpp
//!
use crate::arm_cpu::ArmCpu;
use crate::arm_table;
use crate::instruction_table::{ARMInstr, ThumbInstr};

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

/// Interprets a Thumb instruction
pub fn thumb_interpret(cpu: &mut ArmCpu) {
    todo!()
}

/// Decodes an ARM instruction into an enum or struct
pub fn arm_decode(instruction: u32) -> ARMInstr {
    todo!()
}

/// Decodes a Thumb instruction into an enum or struct
pub fn thumb_decode(instruction: u32) -> ThumbInstr {
    todo!()
}

// ------- States

/// Loads or stores a value using a shifted register addressing mode
pub fn load_store_shift_reg(cpu: &mut ArmCpu, instruction: u32) -> u32 {
    todo!()
}

/// Undefined instruction handler
pub fn undefined(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Data processing instruction
pub fn data_processing(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Counts the leading zeros in a value
pub fn count_leading_zeros(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Saturated operation
pub fn saturated_op(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Multiply instruction
pub fn multiply(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Long multiply instruction
pub fn multiply_long(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Signed halfword multiply instruction
pub fn signed_halfword_multiply(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Swap instruction
pub fn swap(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Store a word to memory
pub fn store_word(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Load a word from memory
pub fn load_word(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Store a byte to memory
pub fn store_byte(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Load a byte from memory
pub fn load_byte(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Store a halfword to memory
pub fn store_halfword(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Load a halfword from memory
pub fn load_halfword(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Load a signed byte from memory
pub fn load_signed_byte(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Load a signed halfword from memory
pub fn load_signed_halfword(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Store a doubleword to memory
pub fn store_doubleword(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Load a doubleword from memory
pub fn load_doubleword(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Store a block of registers to memory
pub fn store_block(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Load a block of registers from memory
pub fn load_block(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Branch instruction
pub fn branch(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Branch with link instruction
pub fn branch_link(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Branch and exchange instruction
pub fn branch_exchange(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Coprocessor register transfer
pub fn coprocessor_reg_transfer(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Branch with link and exchange (register)
pub fn blx_reg(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Branch with link and exchange (immediate)
pub fn blx(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Software interrupt
pub fn swi(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

// ---------------- Thumb

/// Thumb instruction: MOV with shift
pub fn thumb_mov_shift(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: ADD register
pub fn thumb_add_reg(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: SUB register
pub fn thumb_sub_reg(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: MOV immediate
pub fn thumb_mov(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: CMP
pub fn thumb_cmp(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: ADD
pub fn thumb_add(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: SUB
pub fn thumb_sub(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: ALU operations
pub fn thumb_alu_op(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: High register operations
pub fn thumb_hi_reg_op(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: PC-relative load
pub fn thumb_pc_rel_load(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Store register with offset
pub fn thumb_store_reg_offset(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Load register with offset
pub fn thumb_load_reg_offset(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Load halfword
pub fn thumb_load_halfword(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Store halfword
pub fn thumb_store_halfword(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Store with immediate offset
pub fn thumb_store_imm_offset(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Load with immediate offset
pub fn thumb_load_imm_offset(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Load/store signed halfword
pub fn thumb_load_store_sign_halfword(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Stack pointer-relative store
pub fn thumb_sp_rel_store(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Stack pointer-relative load
pub fn thumb_sp_rel_load(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Offset SP operation
pub fn thumb_offset_sp(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Load address
pub fn thumb_load_address(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: PUSH
pub fn thumb_push(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: POP
pub fn thumb_pop(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Store multiple registers
pub fn thumb_store_multiple(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Load multiple registers
pub fn thumb_load_multiple(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Branch
pub fn thumb_branch(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Conditional branch
pub fn thumb_cond_branch(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Prepare long branch
pub fn thumb_long_branch_prep(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Long branch
pub fn thumb_long_branch(cpu: &mut ArmCpu) {
    todo!()
}

/// Thumb instruction: Long branch with link and exchange
pub fn thumb_long_blx(cpu: &mut ArmCpu) {
    todo!()
}
