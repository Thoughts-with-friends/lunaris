// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! cpuinsters.hpp
//!

use crate::arm_cpu::ArmCpu;
use crate::instruction_table::ThumbInstr;

/// Interprets a Thumb instruction
pub fn thumb_interpret(cpu: &mut ArmCpu) {
    todo!()
}

/// Decodes a Thumb instruction into an enum or struct
pub fn thumb_decode(instruction: u32) -> ThumbInstr {
    todo!()
}

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
