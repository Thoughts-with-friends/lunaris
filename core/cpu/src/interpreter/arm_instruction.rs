use crate::arm_cpu::ArmCpu;

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
