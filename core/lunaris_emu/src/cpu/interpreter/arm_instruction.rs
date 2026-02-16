#![allow(clippy::missing_const_for_fn)]
use crate::cpu::arm_cpu::{CpuType, PsrMode, REG_LR, REG_PC, add_overflow, sub_overflow};
use crate::emulator::Emulator;

/// Loads or stores a value using a shifted register addressing mode.
///
/// This implements the ARM *Single Data Transfer* instruction variant where
/// the offset is supplied by a register optionally shifted by an immediate.
///
/// ## Instruction format (I = 1)
///
/// ```text
/// 31          28 27 26 25 24 23 22 21 20 19      16 15      12 11            0
/// +--------------+-----+--+--+--+--+--+--+----------+----------+-------------+
/// |   cond[4]    | 01  | I| P| U| B| W| L|    Rn    |    Rd    |  Offset     |
/// +--------------+-----+--+--+--+--+--+--+----------+----------+-------------+
///
/// I = 1 → Register offset with shift
/// L = 1 → Load, L = 0 → Store
/// B = 1 → Byte, B = 0 → Word
/// ```
///
/// ## Offset (register + shift)
///
/// ```text
/// 11      7 6    5 4      0
/// +--------+------+--------+
/// | shift  | type |  Rm    |
/// +--------+------+--------+
///
/// shift type:
///   00 = Logical Shift Left (LSL)
///   01 = Logical Shift Right (LSR)
///   10 = Arithmetic Shift Right (ASR)
///   11 = Rotate Right (ROR / RRX)
/// ```
///
/// Special cases:
/// - LSR #0 and ASR #0 are interpreted as a shift of 32
/// - ROR #0 performs an RRX (rotate right with carry)
///
/// ## Address calculation
///
/// ```text
/// offset = shift(Rm)
///
/// if U == 1:
///     address = Rn + offset
/// else:
///     address = Rn - offset
///
/// if P == 1:
///     access address
///     if W == 1: Rn = address
/// else:
///     access Rn
///     Rn = address
/// ```
///
/// ## Side effects
/// - May update base register Rn depending on P/W bits
/// - Updates memory or destination register Rd
/// - Returns the effective address used for the transfer
///
/// ## References
/// Single Data Transfer — Register Offset
#[allow(clippy::missing_const_for_fn)]
pub fn load_store_shift_reg(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) -> u32 {
    let mut reg = emu
        .get_cpu_mut(cpu_type)
        .get_register((instruction & 0xF) as i32);

    let shift_type = (instruction >> 5) & 0x3;
    let mut shift = (instruction >> 7) & 0x1F;

    match shift_type {
        0 => {
            // Logical shift left
            reg = emu.get_cpu_mut(cpu_type).lsl(reg, shift as i32, false);
        }
        1 => {
            // Logical shift right
            if shift == 0 {
                shift = 32;
            }
            reg = emu.get_cpu_mut(cpu_type).lsr(reg, shift as i32, false);
        }
        2 => {
            // Arithmetic shift right
            if shift == 0 {
                shift = 32;
            }
            reg = emu.get_cpu_mut(cpu_type).asr(reg, shift as i32, false);
        }
        3 => {
            // Rotate right
            if shift == 0 {
                reg = emu.get_cpu_mut(cpu_type).rrx(reg, false);
            } else {
                reg = emu.get_cpu_mut(cpu_type).rotr32(reg, shift, false);
            }
        }
        _ => {
            #[cfg(feature = "tracing")]
            tracing::warn!("Invalid load/store shift: {shift_type}");
            return 0;
        }
    }

    reg
}

/// Undefined instruction handler
pub fn undefined(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let _ = instruction;
    #[cfg(feature = "tracing")]
    tracing::warn!("Unrecognized ARM opcode {instruction:08X}");

    emu.get_cpu_mut(cpu_type).handle_undefined();
}

/// Data processing instruction
#[allow(clippy::missing_const_for_fn)]
pub fn data_processing(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let opcode = (instruction >> 21) & 0xF;

    let first_operand = ((instruction >> 16) & 0xF) as i32;
    let first_operand_contents = emu.get_cpu_mut(cpu_type).get_register(first_operand);

    let set_condition_codes = (instruction & (1 << 20)) != 0;
    let destination = (instruction >> 12) & 0xF;

    let is_operand_imm = (instruction & (1 << 25)) != 0;

    let set_carry =
        matches!(opcode, 0x0 | 0x1 | 0x8 | 0x9 | 0xC | 0xD | 0xE | 0xF) && set_condition_codes;

    let second_operand = if is_operand_imm {
        // Immediate operand (rotated right)
        let imm = instruction & 0xFF;
        let shift = (instruction & 0xF00) >> 7;
        emu.get_cpu_mut(cpu_type).rotr32(imm, shift, set_carry)
    } else {
        let rm = (instruction & 0xF) as i32;
        let mut value = emu.get_cpu_mut(cpu_type).get_register(rm);
        let shift_type = (instruction >> 5) & 0x3;

        let shift = if (instruction & (1 << 4)) != 0 {
            // Shift by register
            let rs = ((instruction >> 8) & 0xF) as i32;
            emu.get_cpu_mut(cpu_type).add_internal_cycles(1);

            if rm == (REG_PC as i32) {
                value = emu.get_cpu(cpu_type).get_pc() + 4;
            }

            emu.get_cpu_mut(cpu_type).get_register(rs)
        } else {
            // Shift by immediate
            (instruction >> 7) & 0x1F
        } as i32;

        match shift_type {
            0 => emu.get_cpu_mut(cpu_type).lsl(value, shift, set_carry), // LSL
            1 => {
                if shift != 0 || (instruction & (1 << 4)) != 0 {
                    emu.get_cpu_mut(cpu_type).lsr(value, shift, set_carry)
                } else {
                    emu.get_cpu_mut(cpu_type).lsr_32(value, set_carry)
                }
            }
            2 => {
                if shift != 0 || (instruction & (1 << 4)) != 0 {
                    emu.get_cpu_mut(cpu_type).asr(value, shift, set_carry)
                } else {
                    emu.get_cpu_mut(cpu_type).asr_32(value, set_carry)
                }
            }
            3 => {
                if shift == 0 {
                    emu.get_cpu_mut(cpu_type).rrx(value, set_carry)
                } else {
                    emu.get_cpu_mut(cpu_type)
                        .rotr32(value, shift as u32, set_carry)
                }
            }
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!(
                    "Invalid data processing shift type: {shift_type} (instr={instruction:#010X})",
                );
                value
            }
        }
    };

    match opcode {
        0x0 => emu.get_cpu_mut(cpu_type).andd(
            destination as i32,
            first_operand_contents as i32,
            second_operand as i32,
            set_condition_codes,
        ),
        0x1 => emu.get_cpu_mut(cpu_type).eor(
            destination as i32,
            first_operand_contents as i32,
            second_operand as i32,
            set_condition_codes,
        ),
        0x2 => emu.get_cpu_mut(cpu_type).sub(
            destination,
            first_operand_contents,
            second_operand,
            set_condition_codes,
        ),
        0x3 => emu.get_cpu_mut(cpu_type).sub(
            destination,
            second_operand,
            first_operand_contents,
            set_condition_codes,
        ), // RSB
        0x4 => emu.get_cpu_mut(cpu_type).add(
            destination,
            first_operand_contents,
            second_operand,
            set_condition_codes,
        ),
        0x5 => emu.get_cpu_mut(cpu_type).adc(
            destination,
            first_operand_contents,
            second_operand,
            set_condition_codes,
        ),
        0x6 => emu.get_cpu_mut(cpu_type).sbc(
            destination,
            first_operand_contents,
            second_operand,
            set_condition_codes,
        ),
        0x7 => emu.get_cpu_mut(cpu_type).sbc(
            destination,
            second_operand,
            first_operand_contents,
            set_condition_codes,
        ), // RSC

        0x8 => {
            if set_condition_codes {
                emu.get_cpu_mut(cpu_type)
                    .tst(first_operand_contents, second_operand);
            } else {
                emu.get_cpu_mut(cpu_type).mrs(instruction);
            }
        }
        0x9 => {
            if set_condition_codes {
                emu.get_cpu_mut(cpu_type)
                    .teq(first_operand_contents, second_operand);
            } else {
                emu.get_cpu_mut(cpu_type).msr(instruction);
            }
        }
        0xA => {
            if set_condition_codes {
                emu.get_cpu_mut(cpu_type)
                    .cmp(first_operand_contents, second_operand);
            } else {
                emu.get_cpu_mut(cpu_type).mrs(instruction);
            }
        }
        0xB => {
            if set_condition_codes {
                emu.get_cpu_mut(cpu_type)
                    .cmn(first_operand_contents, second_operand);
            } else {
                emu.get_cpu_mut(cpu_type).msr(instruction);
            }
        }

        0xC => emu.get_cpu_mut(cpu_type).orr(
            destination as i32,
            first_operand_contents as i32,
            second_operand as i32,
            set_condition_codes,
        ),
        0xD => emu
            .get_cpu_mut(cpu_type)
            .mov(destination, second_operand, set_condition_codes),
        0xE => emu.get_cpu_mut(cpu_type).bic(
            destination,
            first_operand_contents,
            second_operand,
            set_condition_codes,
        ),
        0xF => emu
            .get_cpu_mut(cpu_type)
            .mvn(destination, second_operand, set_condition_codes),

        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!(
                "Unrecognized data processing opcode: {:#X} (instr={:#010X})",
                opcode,
                instruction
            );
        }
    }
}

/// Counts the leading zeros in a value
pub fn count_leading_zeros(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    // CLZ is undefined when ID flag is set
    if emu.get_cpu(cpu_type).get_id() <= 0 {
        #[cfg(feature = "tracing")]
        tracing::error!("CLZ executed while ID flag set (instr={instruction:#010X})");

        emu.get_cpu_mut(cpu_type).handle_undefined();
        return;
    }

    let source_reg = (instruction & 0xF) as i32;
    let destination = ((instruction >> 12) & 0xF) as i32;

    // if emu.get_cpu_mut(cpu_type).can_disassemble() { ... }

    let mut source = emu.get_cpu_mut(cpu_type).get_register(source_reg);

    // Implementation lifted from melonDS (matches original behavior)
    let mut bits: u32 = 0;

    while (source & 0xFF00_0000) == 0 {
        bits += 8;
        source <<= 8;
        source |= 0xFF;
    }

    while (source & 0x8000_0000) == 0 {
        bits += 1;
        source <<= 1;
        source |= 1;
    }

    emu.get_cpu_mut(cpu_type).set_register(destination, bits);
}

/// Saturated operation
pub fn saturated_op(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    // Saturated ops are undefined when ID flag is set
    if emu.get_cpu(cpu_type).get_id() <= 0 {
        #[cfg(feature = "tracing")]
        tracing::error!("Saturated op executed while ID flag set (instr={instruction:#010X})");

        emu.get_cpu_mut(cpu_type).handle_undefined();
        return;
    }

    let opcode = (instruction >> 20) & 0xF;
    let operand = (instruction >> 16) & 0xF;
    let destination = (instruction >> 12) & 0xF;
    let source = (instruction & 0xF) as usize;

    let operand_reg = emu.get_cpu_mut(cpu_type).get_register(operand as i32);
    let source_reg = emu.get_cpu_mut(cpu_type).get_register(source as i32);

    let mut result: u32;

    match opcode {
        0x0 => {
            // QADD
            result = operand_reg.wrapping_add(source_reg);

            if add_overflow(source_reg, operand_reg, result) {
                emu.get_cpu_mut(cpu_type).get_cpsr_mut().sticky_overflow = true;

                result = if (result & 0x8000_0000) != 0 {
                    0x7FFF_FFFF
                } else {
                    0x8000_0000
                };
            }

            emu.get_cpu_mut(cpu_type)
                .set_register(destination as i32, result);
        }

        0x2 => {
            // QSUB
            result = source_reg.wrapping_sub(operand_reg);

            if sub_overflow(source_reg, operand_reg, result) {
                emu.get_cpu_mut(cpu_type).get_cpsr_mut().sticky_overflow = true;

                result = if (result & 0x8000_0000) != 0 {
                    0x7FFF_FFFF
                } else {
                    0x8000_0000
                };
            }

            emu.get_cpu_mut(cpu_type)
                .set_register(destination as i32, result);
        }

        // QDADD / QDSUB intentionally unimplemented (same as C++)
        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!("Unrecognized saturated opcode {opcode} (instr={instruction:#010X})");
        }
    }
}

/// Multiply instruction
#[allow(clippy::missing_const_for_fn)]
pub fn multiply(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let accumulate = instruction & (1 << 21);
    let set_condition_codes = instruction & (1 << 20);
    let destination = ((instruction >> 16) & 0xF) as i32;
    let first_operand = (instruction & 0xF) as i32;
    let second_operand = ((instruction >> 8) & 0xF) as i32;
    let third_operand = ((instruction >> 12) & 0xF) as i32;

    let mut result = emu.get_cpu_mut(cpu_type).get_register(first_operand)
        * emu.get_cpu_mut(cpu_type).get_register(second_operand);

    if accumulate != 0 {
        //if (emu.get_cpu_mut(cpu_type).can_disassemble())
        #[cfg(feature = "tracing")]
        tracing::error!("MLA {destination}, {first_operand}, {second_operand}, {third_operand}");
        result += emu.get_cpu_mut(cpu_type).get_register(third_operand);
    }
    //else if (emu.get_cpu_mut(cpu_type).can_disassemble())
    #[cfg(feature = "tracing")]
    tracing::error!("MUL {destination}, {first_operand}, {second_operand}");

    if set_condition_codes != 0 {
        emu.get_cpu_mut(cpu_type).set_zero_neg_flags(result);
    }

    emu.get_cpu_mut(cpu_type).set_register(destination, result);
}

/// Long multiply instruction
pub fn multiply_long(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let is_signed = (instruction & (1 << 22)) != 0;
    let accumulate = (instruction & (1 << 21)) != 0;
    let set_condition_codes = (instruction & (1 << 20)) != 0;

    let dest_hi = ((instruction >> 16) & 0xF) as i32;
    let dest_lo = ((instruction >> 12) & 0xF) as i32;
    let first_operand = ((instruction >> 8) & 0xF) as i32;
    let second_operand = (instruction & 0xF) as i32;

    let first_operand = emu.get_cpu_mut(cpu_type).get_register(first_operand);
    let second_operand = emu.get_cpu_mut(cpu_type).get_register(second_operand);

    if is_signed {
        // Signed long multiply
        let mut result = (first_operand as i32 as i64) * (second_operand as i32 as i64);

        if accumulate {
            let mut big_reg = emu.get_cpu_mut(cpu_type).get_register(dest_lo) as i64;
            big_reg |= (emu.get_cpu_mut(cpu_type).get_register(dest_hi) as i64) << 32;
            result = result.wrapping_add(big_reg);
        }

        emu.get_cpu_mut(cpu_type)
            .set_register(dest_lo, result as u32);
        emu.get_cpu_mut(cpu_type)
            .set_register(dest_hi, (result >> 32) as u32);

        if set_condition_codes {
            emu.get_cpu_mut(cpu_type).set_zero(result == 0);
            emu.get_cpu_mut(cpu_type).set_neg(((result >> 63) & 1) != 0);
        }
    } else {
        // Unsigned long multiply
        let mut result = (first_operand as u64) * (second_operand as u64);

        if accumulate {
            let mut big_reg = emu.get_cpu_mut(cpu_type).get_register(dest_lo) as u64;
            big_reg |= (emu.get_cpu_mut(cpu_type).get_register(dest_hi) as u64) << 32;
            result = result.wrapping_add(big_reg);
        }

        emu.get_cpu_mut(cpu_type)
            .set_register(dest_lo, result as u32);
        emu.get_cpu_mut(cpu_type)
            .set_register(dest_hi, (result >> 32) as u32);

        if set_condition_codes {
            emu.get_cpu_mut(cpu_type).set_zero(result == 0);
            emu.get_cpu_mut(cpu_type).set_neg(((result >> 63) & 1) != 0);
        }
    }
}

/// Signed halfword multiply instruction
pub fn signed_halfword_multiply(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    // No-op if ID bit set (matches C++)
    if emu.get_cpu(cpu_type).get_id() > 0 {
        return;
    }

    let destination = ((instruction >> 16) & 0xF) as i32;
    let accumulate = ((instruction >> 12) & 0xF) as i32;
    let first_operand = ((instruction >> 8) & 0xF) as i32;
    let second_operand = (instruction & 0xF) as i32;
    let opcode = (instruction >> 21) & 0xF;

    let first_op_top = (instruction & (1 << 6)) != 0;
    let second_op_top = (instruction & (1 << 5)) != 0;

    let mut product: i32;
    let result: u32;

    match opcode {
        0x8 => {
            // SMLAxy
            let op1 = emu.get_cpu_mut(cpu_type).get_register(first_operand);
            let op2 = emu.get_cpu_mut(cpu_type).get_register(second_operand);

            product = if first_op_top {
                (op1 >> 16) as i16 as i32
            } else {
                (op1 & 0xFFFF) as i16 as i32
            };

            let rhs = if second_op_top {
                (op2 >> 16) as i16 as i32
            } else {
                (op2 & 0xFFFF) as i16 as i32
            };

            product *= rhs;

            let acc = emu.get_cpu_mut(cpu_type).get_register(accumulate);
            let sum = (product as i64) + (acc as i64);
            result = sum as u32;

            if add_overflow(product as u32, acc, result) {
                emu.get_cpu_mut(cpu_type).get_cpsr_mut().sticky_overflow = true;
            }
        }

        0x9 => {
            // SMULWy / SMLAWy
            let op1 = emu.get_cpu_mut(cpu_type).get_register(first_operand);
            let op2 = emu.get_cpu_mut(cpu_type).get_register(second_operand);

            product = if first_op_top {
                (op1 >> 16) as i16 as i32
            } else {
                (op1 & 0xFFFF) as i16 as i32
            };

            let big_product = (product as i64 * op2 as i32 as i64) >> 16;

            if !second_op_top {
                // SMLAWy
                let acc = emu.get_cpu_mut(cpu_type).get_register(accumulate);
                let sum = big_product + acc as i64;
                result = sum as u32;

                if add_overflow(big_product as u32, acc, result) {
                    emu.get_cpu_mut(cpu_type).get_cpsr_mut().sticky_overflow = true;
                }
            } else {
                // SMULWy
                result = big_product as u32;
            }
        }

        0xB => {
            // SMULxy
            let op1 = emu.get_cpu_mut(cpu_type).get_register(first_operand);
            let op2 = emu.get_cpu_mut(cpu_type).get_register(second_operand);

            product = if first_op_top {
                (op1 >> 16) as i16 as i32
            } else {
                (op1 & 0xFFFF) as i16 as i32
            };

            let rhs = if second_op_top {
                (op2 >> 16) as i16 as i32
            } else {
                (op2 & 0xFFFF) as i16 as i32
            };

            product *= rhs;
            result = product as u32;
        }

        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!("Unrecognized signed halfword multiply opcode {opcode:X}");
            return;
        }
    }

    emu.get_cpu_mut(cpu_type).set_register(destination, result);
}

/// Swap instruction
pub fn swap(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let is_byte = (instruction & (1 << 22)) != 0;
    let base = ((instruction >> 16) & 0xF) as i32;
    let destination = ((instruction >> 12) & 0xF) as i32;
    let source = (instruction & 0xF) as i32;

    let address = emu.get_cpu_mut(cpu_type).get_register(base);

    if is_byte {
        // SWPB
        let byte = emu.read_byte(address, cpu_type);

        let value = (emu.get_cpu_mut(cpu_type).get_register(source) & 0xFF) as u8;
        emu.write_byte(address, value, cpu_type);

        let arm = emu.get_cpu_mut(cpu_type);
        arm.set_register(destination, byte as u32);
        arm.add_n16_data(address, 2);
    } else {
        // SWP (word)
        let aligned = address & !0x3;
        let rotate = (address & 0x3) * 8;

        let value = emu.read_word(aligned, cpu_type);
        let word = emu.get_cpu_mut(cpu_type).rotr32(value, rotate, false);

        let value = emu.get_cpu_mut(cpu_type).get_register(source);
        emu.write_word(address, value, cpu_type);

        let arm = emu.get_cpu_mut(cpu_type);
        arm.set_register(destination, word);
        arm.add_n32_data(address, 2);
    }
}

/// Store a word to memory
pub fn store_word(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let base = (instruction >> 16) & 0xF;
    let source = (instruction >> 12) & 0xF;

    let is_imm = (instruction & (1 << 25)) == 0;
    let is_preindexing = (instruction & (1 << 24)) != 0;
    let is_adding_offset = (instruction & (1 << 23)) != 0;
    let is_writing_back = (instruction & (1 << 21)) != 0;

    let offset: u32 = if is_imm {
        instruction & 0xFFF
    } else {
        load_store_shift_reg(emu, cpu_type, instruction)
    };

    let mut address = emu.get_cpu(cpu_type).get_register(base as i32);
    let value = emu.get_cpu(cpu_type).get_register(source as i32);

    if is_preindexing {
        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        if is_writing_back {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }

        emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);
        emu.write_word(address & !0x3, value, cpu_type);
    } else {
        emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);
        emu.write_word(address & !0x3, value, cpu_type);

        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
    }
}

/// Load a word from memory
pub fn load_word(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let base = (instruction >> 16) & 0xF;
    let destination = (instruction >> 12) & 0xF;

    let is_imm = (instruction & (1 << 25)) == 0;
    let is_pre_indexing = (instruction & (1 << 24)) != 0;
    let is_adding_offset = (instruction & (1 << 23)) != 0;
    let is_writing_back = (instruction & (1 << 21)) != 0;

    let offset: u32 = if is_imm {
        instruction & 0xFFF
    } else {
        load_store_shift_reg(emu, cpu_type, instruction)
    };

    let mut address = emu.get_cpu(cpu_type).get_register(base as i32);
    emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);

    if is_pre_indexing {
        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        if is_writing_back {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }

        let value = emu.read_word(address & !0x3, cpu_type);
        let word = emu
            .get_cpu_mut(cpu_type)
            .rotr32(value, (address & 0x3) * 8, false);

        if destination == REG_PC {
            // Only ARM9 can change thumb state
            let has_change_cpu_id = emu.get_cpu(cpu_type).get_id() <= 0;
            emu.get_cpu_mut(cpu_type).jp(word, has_change_cpu_id);
        } else {
            emu.get_cpu_mut(cpu_type)
                .set_register(destination as i32, word);
        }
    } else {
        let value = emu.read_word(address & !0x3, cpu_type);
        let word = emu
            .get_cpu_mut(cpu_type)
            .rotr32(value, (address & 0x3) * 8, false);

        if destination == REG_PC {
            // Only ARM9 can change thumb state
            let has_change_cpu_id = emu.get_cpu(cpu_type).get_id() <= 0;
            emu.get_cpu_mut(cpu_type).jp(word, has_change_cpu_id);
        } else {
            emu.get_cpu_mut(cpu_type)
                .set_register(destination as i32, word);
        }

        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        if base != destination {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }
    }
}

/// Store a byte to memory
pub fn store_byte(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let base = (instruction >> 16) & 0xF;
    let source = (instruction >> 12) & 0xF;

    let is_imm = (instruction & (1 << 25)) == 0;
    let is_preindexing = (instruction & (1 << 24)) != 0;
    let is_adding_offset = (instruction & (1 << 23)) != 0;
    let is_writing_back = (instruction & (1 << 21)) != 0;

    let offset: u32 = if is_imm {
        instruction & 0xFFF
    } else {
        load_store_shift_reg(emu, cpu_type, instruction)
    };

    let mut address = emu.get_cpu(cpu_type).get_register(base as i32);
    let value = (emu.get_cpu(cpu_type).get_register(source as i32) & 0xFF) as u8;

    emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);

    if is_preindexing {
        match is_adding_offset {
            true => address = address.wrapping_add(offset),
            false => address = address.wrapping_sub(offset),
        }

        if is_writing_back {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }
        emu.write_byte(address, value, cpu_type);
    } else {
        emu.write_byte(address, value, cpu_type);

        match is_adding_offset {
            true => address = address.wrapping_add(offset),
            false => address = address.wrapping_sub(offset),
        }

        emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
    }
}

/// Load a byte from memory
pub fn load_byte(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let base = (instruction >> 16) & 0xF;
    let destination = (instruction >> 12) & 0xF;

    let is_imm = (instruction & (1 << 25)) == 0;
    let is_preindexing = (instruction & (1 << 24)) != 0;
    let is_adding_offset = (instruction & (1 << 23)) != 0;
    let is_writing_back = (instruction & (1 << 21)) != 0;

    let offset: u32 = if is_imm {
        instruction & 0xFFF
    } else {
        load_store_shift_reg(emu, cpu_type, instruction)
    };

    let mut address = emu.get_cpu(cpu_type).get_register(base as i32);

    // Timing behavior matches original C++:
    emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
    emu.get_cpu_mut(cpu_type).add_internal_cycles(1);

    if is_preindexing {
        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        if is_writing_back {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }

        let value = emu.read_byte(address, cpu_type) as u32;
        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, value);
    } else {
        let value = emu.read_byte(address, cpu_type) as u32;
        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, value);

        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        if base != destination {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }
    }
}

/// Store a halfword to memory
pub fn store_halfword(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let is_preindexing = (instruction & (1 << 24)) != 0;
    let is_adding_offset = (instruction & (1 << 23)) != 0;
    let is_writing_back = (instruction & (1 << 21)) != 0;

    let base = (instruction >> 16) & 0xF;
    let source = (instruction >> 12) & 0xF;

    let is_imm_offset = (instruction & (1 << 22)) != 0;

    let mut offset = instruction & 0xF;
    if is_imm_offset {
        offset |= (instruction >> 4) & 0xF0;
    } else {
        offset = emu.get_cpu(cpu_type).get_register(offset as i32);
    }

    let mut address = emu.get_cpu(cpu_type).get_register(base as i32);
    let halfword = (emu.get_cpu(cpu_type).get_register(source as i32) & 0xFFFF) as u16;

    if is_preindexing {
        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        emu.write_halfword(address, halfword, cpu_type);

        if is_writing_back {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }
    } else {
        emu.write_halfword(address, halfword, cpu_type);

        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
    }

    emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
}

/// Load a halfword from memory
pub fn load_halfword(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let is_preindexing = (instruction & (1 << 24)) != 0;
    let is_adding_offset = (instruction & (1 << 23)) != 0;
    let is_writing_back = (instruction & (1 << 21)) != 0;

    let base = (instruction >> 16) & 0xF;
    let destination = (instruction >> 12) & 0xF;

    let is_imm_offset = (instruction & (1 << 22)) != 0;

    let mut offset = instruction & 0xF;
    if is_imm_offset {
        offset |= (instruction >> 4) & 0xF0;
    } else {
        offset = emu.get_cpu(cpu_type).get_register(offset as i32);
    }

    let mut address = emu.get_cpu(cpu_type).get_register(base as i32);

    if is_preindexing {
        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        if is_writing_back && base != destination {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }

        let value = emu.read_halfword(address, cpu_type) as u32;
        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, value);
    } else {
        let value = emu.read_halfword(address, cpu_type) as u32;
        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, value);

        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        if base != destination {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }
    }

    emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
}

/// Load a signed byte from memory
pub fn load_signed_byte(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let is_preindexing = (instruction & (1 << 24)) != 0;
    let is_adding_offset = (instruction & (1 << 23)) != 0;
    let is_writing_back = (instruction & (1 << 21)) != 0;

    let base = (instruction >> 16) & 0xF;
    let destination = (instruction >> 12) & 0xF;

    let is_imm_offset = (instruction & (1 << 22)) != 0;

    let mut offset = instruction & 0xF;
    if is_imm_offset {
        offset |= (instruction >> 4) & 0xF0;
    } else {
        offset = emu.get_cpu(cpu_type).get_register(offset as i32);
    }

    let mut address = emu.get_cpu(cpu_type).get_register(base as i32);

    if is_preindexing {
        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        if is_writing_back {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }

        let byte = emu.read_byte(address, cpu_type) as i8;
        let value = byte as i32 as u32;
        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, value);
    } else {
        let byte = emu.read_byte(address, cpu_type) as i8;
        let value = byte as i32 as u32;
        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, value);

        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        if base != destination {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }
    }

    emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
}

/// Load a signed halfword from memory
pub fn load_signed_halfword(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let is_preindexing = (instruction & (1 << 24)) != 0;
    let is_adding_offset = (instruction & (1 << 23)) != 0;
    let is_writing_back = (instruction & (1 << 21)) != 0;

    let base = (instruction >> 16) & 0xF;
    let destination = (instruction >> 12) & 0xF;

    let is_imm_offset = (instruction & (1 << 22)) != 0;

    let mut offset = instruction & 0xF;
    if is_imm_offset {
        offset |= (instruction >> 4) & 0xF0;
    } else {
        offset = emu.get_cpu(cpu_type).get_register(offset as i32);
    }

    let mut address = emu.get_cpu(cpu_type).get_register(base as i32);

    if is_preindexing {
        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        if is_writing_back {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }

        let half = emu.read_halfword(address, cpu_type) as i16;
        let value = half as i32 as u32;
        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, value);
    } else {
        let half = emu.read_halfword(address, cpu_type) as i16;
        let value = half as i32 as u32;
        emu.get_cpu_mut(cpu_type)
            .set_register(destination as i32, value);

        if is_adding_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        if base != destination {
            emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
        }
    }

    emu.get_cpu_mut(cpu_type).add_n16_data(address, 1);
}

/// Store a doubleword to memory
pub fn store_doubleword(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    // Only supported on ARM9 (matches C++ behavior)
    if emu.get_cpu(cpu_type).get_id() != 0 {
        emu.get_cpu_mut(cpu_type).handle_undefined();
        return;
    }

    let is_preindexing = (instruction & (1 << 24)) != 0;
    let add_offset = (instruction & (1 << 23)) != 0;
    let is_imm_offset = (instruction & (1 << 22)) != 0;
    let write_back = (instruction & (1 << 21)) != 0;

    let base = (instruction >> 16) & 0xF;
    let source = (instruction >> 12) & 0xF;

    let mut offset = instruction & 0xF;
    if is_imm_offset {
        offset |= (instruction >> 4) & 0xF0;
    } else {
        offset = emu.get_cpu(cpu_type).get_register(offset as i32);
    }

    let mut address = emu.get_cpu(cpu_type).get_register(base as i32);

    if is_preindexing {
        if add_offset {
            address = address.wrapping_add(offset);
        } else {
            address = address.wrapping_sub(offset);
        }

        let low = emu.get_cpu(cpu_type).get_register(source as i32);
        let high = emu.get_cpu(cpu_type).get_register((source + 1) as i32);

        emu.write_word(address, low, cpu_type);
        emu.write_word(address.wrapping_add(4), high, cpu_type);

        if write_back {
            // Mirrors the original C++ exactly (even though it looks odd)
            let base_reg = emu.get_cpu(cpu_type).get_register(base as i32);
            emu.get_cpu_mut(cpu_type)
                .set_register(base_reg as i32, address.wrapping_add(4));
        }
    } else {
        #[cfg(feature = "tracing")]
        tracing::error!("store post_indexing not supported");
    }
}

/// Load multiple registers from memory (LDM)
pub fn load_block(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let reg_list = instruction & 0xFFFF;
    let base = (instruction >> 16) & 0xF;

    let is_writing_back = (instruction & (1 << 21)) != 0;
    let load_psr = (instruction & (1 << 22)) != 0;
    let is_adding_offset = (instruction & (1 << 23)) != 0;
    let is_preindexing = (instruction & (1 << 24)) != 0;

    let user_bank_transfer = load_psr && (reg_list & (1 << 15)) == 0;

    let mut address = emu.get_cpu(cpu_type).get_register(base as i32);

    let offset: i32 = if is_adding_offset { 4 } else { -4 };

    // Switch to USER bank if required
    let old_mode = emu.get_cpu_mut(cpu_type).get_cpsr().mode;

    if user_bank_transfer {
        emu.get_cpu_mut(cpu_type).update_reg_mode(PsrMode::User);
        emu.get_cpu_mut(cpu_type).get_cpsr_mut().mode = PsrMode::User;
    }

    let mut regs = 0;

    if is_adding_offset {
        // Incrementing
        for i in 0..15 {
            if (reg_list & (1 << i)) != 0 {
                regs += 1;

                if is_preindexing {
                    address = address.wrapping_add(offset as u32);
                    let value = emu.read_word(address, cpu_type);
                    emu.get_cpu_mut(cpu_type).set_register(i, value);
                } else {
                    let value = emu.read_word(address, cpu_type);
                    emu.get_cpu_mut(cpu_type).set_register(i, value);
                    address = address.wrapping_add(offset as u32);
                }
            }
        }

        // PC (R15) handled last
        if (reg_list & (1 << 15)) != 0 {
            if is_preindexing {
                address = address.wrapping_add(offset as u32);
                let mut new_pc = emu.read_word(address, cpu_type);
                if emu.get_cpu(cpu_type).get_id() != 0 {
                    new_pc &= !0x1;
                }
                emu.get_cpu_mut(cpu_type).jp(new_pc, true);
            } else {
                let mut new_pc = emu.read_word(address, cpu_type);
                if emu.get_cpu(cpu_type).get_id() != 0 {
                    new_pc &= !0x1;
                }
                emu.get_cpu_mut(cpu_type).jp(new_pc, true);
                address = address.wrapping_add(offset as u32);
            }

            if load_psr {
                emu.get_cpu_mut(cpu_type).spsr_to_cpsr();
            }

            regs += 1;
        }
    } else {
        // Decrementing: PC first
        if (reg_list & (1 << 15)) != 0 {
            if is_preindexing {
                address = address.wrapping_add(offset as u32);
                let mut new_pc = emu.read_word(address, cpu_type);
                if emu.get_cpu(cpu_type).get_id() != 0 {
                    new_pc &= !0x1;
                }
                emu.get_cpu_mut(cpu_type).jp(new_pc, true);
            } else {
                let mut new_pc = emu.read_word(address, cpu_type);
                if emu.get_cpu(cpu_type).get_id() != 0 {
                    new_pc &= !0x1;
                }
                emu.get_cpu_mut(cpu_type).jp(new_pc, true);
                address = address.wrapping_add(offset as u32);
            }

            if load_psr {
                emu.get_cpu_mut(cpu_type).spsr_to_cpsr();
            }

            regs += 1;
        }

        for i in (0..15).rev() {
            if (reg_list & (1 << i)) != 0 {
                regs += 1;

                if is_preindexing {
                    address = address.wrapping_add(offset as u32);
                    let value = emu.read_word(address, cpu_type);
                    emu.get_cpu_mut(cpu_type).set_register(i, value);
                } else {
                    let value = emu.read_word(address, cpu_type);
                    emu.get_cpu_mut(cpu_type).set_register(i, value);
                    address = address.wrapping_add(offset as u32);
                }
            }
        }
    }

    // Restore original mode if USER bank was used
    if user_bank_transfer {
        emu.get_cpu_mut(cpu_type).update_reg_mode(old_mode);
        emu.get_cpu_mut(cpu_type).get_cpsr_mut().mode = old_mode;
    }

    // Timing
    if regs > 1 {
        emu.get_cpu_mut(cpu_type).add_s32_data(address, regs - 1);
    }
    emu.get_cpu_mut(cpu_type).add_n32_data(address, 1);
    emu.get_cpu_mut(cpu_type).add_internal_cycles(1);

    // Writeback (blocked when base is in list on ARM9)
    if is_writing_back && !((reg_list & (1 << base)) != 0 && emu.get_cpu(cpu_type).get_id() != 0) {
        emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
    }
}

/// Store a block of registers to memory
pub fn store_block(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let reg_list = instruction & 0xFFFF;
    let base = (instruction >> 16) & 0xF;

    let is_writing_back = (instruction & (1 << 21)) != 0;
    let load_psr = (instruction & (1 << 22)) != 0;
    let is_adding_offset = (instruction & (1 << 23)) != 0;
    let is_preindexing = (instruction & (1 << 24)) != 0;

    let user_bank_transfer = load_psr && (reg_list & (1 << 15)) == 0;

    let mut address = emu.get_cpu(cpu_type).get_register(base as i32);

    let offset: i32 = if is_adding_offset { 4 } else { -4 };

    // Handle user-mode banked register transfer
    let old_mode = emu.get_cpu_mut(cpu_type).get_cpsr().mode;

    if user_bank_transfer {
        emu.get_cpu_mut(cpu_type).update_reg_mode(PsrMode::User);
        emu.get_cpu_mut(cpu_type).get_cpsr_mut().mode = PsrMode::User;
    }

    let mut regs = 0;

    if is_adding_offset {
        // Incrementing: low → high
        for i in 0..16 {
            if (reg_list & (1 << i)) != 0 {
                regs += 1;

                if is_preindexing {
                    address = address.wrapping_add(offset as u32);
                    let value = emu.get_cpu(cpu_type).get_register(i);
                    emu.write_word(address, value, cpu_type);
                } else {
                    let value = emu.get_cpu(cpu_type).get_register(i);
                    emu.write_word(address, value, cpu_type);
                    address = address.wrapping_add(offset as u32);
                }
            }
        }
    } else {
        // Decrementing: high → low
        for i in (0..16).rev() {
            if (reg_list & (1 << i)) != 0 {
                regs += 1;

                if is_preindexing {
                    address = address.wrapping_add(offset as u32);
                    let value = emu.get_cpu(cpu_type).get_register(i);
                    emu.write_word(address, value, cpu_type);
                } else {
                    let value = emu.get_cpu(cpu_type).get_register(i);
                    emu.write_word(address, value, cpu_type);
                    address = address.wrapping_add(offset as u32);
                }
            }
        }
    }

    // Restore original mode if we switched to USER
    if user_bank_transfer {
        emu.get_cpu_mut(cpu_type).update_reg_mode(old_mode);
        emu.get_cpu_mut(cpu_type).get_cpsr_mut().mode = old_mode;
    }

    // Timing behavior (matches C++)
    if regs > 2 {
        emu.get_cpu_mut(cpu_type).add_s32_data(address, regs - 1);
    }
    emu.get_cpu_mut(cpu_type).add_n32_data(address, 2);

    if is_writing_back {
        emu.get_cpu_mut(cpu_type).set_register(base as i32, address);
    }
}

/// Branch instruction
pub fn branch(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let mut address = emu.get_cpu(cpu_type).get_pc();

    // 24-bit signed offset, shifted left by 2
    let mut offset = ((instruction & 0x00FF_FFFF) << 2) as i32;

    // Sign-extend
    offset <<= 6;
    offset >>= 6;

    address = address.wrapping_add(offset as u32);

    emu.get_cpu_mut(cpu_type).jp(address, false);
}

/// Branch with link instruction
pub fn branch_link(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let address = emu.get_cpu(cpu_type).get_pc();

    // 24-bit signed offset, shifted left by 2
    let mut offset = ((instruction & 0x00FF_FFFF) << 2) as i32;

    // Sign extend
    offset <<= 6;
    offset >>= 6;

    // Branch
    let target = address.wrapping_add(offset as u32);
    emu.get_cpu_mut(cpu_type).jp(target, false);

    // Link register gets address of next instruction minus 4
    emu.get_cpu_mut(cpu_type)
        .set_register(REG_LR as i32, address.wrapping_sub(4));
}

/// Branch and exchange instruction
pub fn branch_exchange(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let reg_id = instruction & 0xF;
    let new_address = emu.get_cpu(cpu_type).get_register(reg_id as i32);

    emu.get_cpu_mut(cpu_type).jp(new_address, true);
}

/// Coprocessor register transfer
pub fn coprocessor_reg_transfer(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let operation_mode = (instruction >> 21) & 0x7;
    let is_loading = (instruction & (1 << 20)) != 0;

    let cp_reg = (instruction >> 16) & 0xF;
    let arm_reg = (instruction >> 12) & 0xF;

    let coprocessor_id = (instruction >> 8) & 0xF;
    let coprocessor_info = (instruction >> 5) & 0x7;
    let coprocessor_operand = instruction & 0xF;

    if is_loading {
        // MRC
        emu.get_cpu_mut(cpu_type).add_internal_cycles(2);
        emu.get_cpu_mut(cpu_type).add_cop_cycles(1);

        match coprocessor_id {
            15 => {
                let value = emu.arm9_cp15.mrc(
                    operation_mode as i32,
                    cp_reg as i32,
                    coprocessor_info as i32,
                    coprocessor_operand as i32,
                );
                emu.get_cpu_mut(cpu_type)
                    .set_register(arm_reg as i32, value);
            }
            _ => {
                // mirrors printf + exit(1)
                #[cfg(feature = "tracing")]
                tracing::error!("Coprocessor {coprocessor_id} not recognized");
            }
        }
    } else {
        // MCR
        emu.get_cpu_mut(cpu_type).add_internal_cycles(1);
        emu.get_cpu_mut(cpu_type).add_cop_cycles(1);

        match coprocessor_id {
            15 => {
                let value = emu.get_cpu_mut(cpu_type).get_register(arm_reg as i32);
                emu.arm9_cp15.mcr(
                    operation_mode as i32,
                    cp_reg as i32,
                    value,
                    coprocessor_info as i32,
                    coprocessor_operand as i32,
                );
            }
            _ => {
                // mirrors printf + exit(1)
                #[cfg(feature = "tracing")]
                tracing::error!("Coprocessor {coprocessor_id} not recognized");
            }
        }
    }
}

/// Branch with Link and Exchange (BLX, register)
pub fn blx_reg(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    // Save return address
    let pc = emu.get_cpu(cpu_type).get_pc();
    emu.get_cpu_mut(cpu_type)
        .set_register(REG_LR as i32, pc.wrapping_sub(4));

    let reg_id = instruction & 0xF;
    let new_address = emu.get_cpu(cpu_type).get_register(reg_id as i32);

    // Branch and exchange
    emu.get_cpu_mut(cpu_type).jp(new_address, true);
}

/// Branch with Link and Exchange (BLX, immediate)
pub fn blx(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let address = emu.get_cpu(cpu_type).get_pc();

    // 24-bit signed offset, shifted left by 2
    let mut offset = ((instruction & 0x00FF_FFFF) << 2) as i32;

    // Sign extend
    offset <<= 6;
    offset >>= 6;

    // H bit adds 2
    if (instruction & (1 << 24)) != 0 {
        offset += 2;
    }

    // Set link register
    emu.get_cpu_mut(cpu_type)
        .set_register(REG_LR as i32, address.wrapping_sub(4));

    // Branch and exchange (force Thumb)
    let target = address.wrapping_add(offset as u32).wrapping_add(1);

    emu.get_cpu_mut(cpu_type).jp(target, true);
}

/// Software interrupt
pub fn swi(emu: &mut Emulator, cpu_type: CpuType, instruction: u32) {
    let _ = instruction;
    emu.get_cpu_mut(cpu_type).handle_swi();
}
