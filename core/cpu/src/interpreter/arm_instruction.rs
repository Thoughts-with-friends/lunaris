#![allow(clippy::missing_const_for_fn)]
use crate::arm_cpu::{ArmCpu, REG_LR, REG_PC, add_overflow, sub_overflow};

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
pub fn load_store_shift_reg(cpu: &mut ArmCpu, instruction: u32) -> u32 {
    let mut reg = cpu.get_register((instruction & 0xF) as i32);

    let shift_type = (instruction >> 5) & 0x3;
    let mut shift = (instruction >> 7) & 0x1F;

    match shift_type {
        0 => {
            // Logical shift left
            reg = cpu.lsl(reg, shift as i32, false);
        }
        1 => {
            // Logical shift right
            if shift == 0 {
                shift = 32;
            }
            reg = cpu.lsr(reg, shift as i32, false);
        }
        2 => {
            // Arithmetic shift right
            if shift == 0 {
                shift = 32;
            }
            reg = cpu.asr(reg, shift as i32, false);
        }
        3 => {
            // Rotate right
            if shift == 0 {
                reg = cpu.rrx(reg, false);
            } else {
                reg = cpu.rotr32(reg, shift, false);
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
pub fn undefined(cpu: &mut ArmCpu, instruction: u32) {
    let _ = instruction;
    #[cfg(feature = "tracing")]
    tracing::warn!("Unrecognized ARM opcode {instruction:08X}");

    cpu.handle_undefined();
}

/// Data processing instruction
#[allow(clippy::missing_const_for_fn)]
pub fn data_processing(cpu: &mut ArmCpu, instruction: u32) {
    let opcode = (instruction >> 21) & 0xF;

    let first_operand = ((instruction >> 16) & 0xF) as i32;
    let first_operand_contents = cpu.get_register(first_operand);

    let set_condition_codes = (instruction & (1 << 20)) != 0;
    let destination = (instruction >> 12) & 0xF;

    let is_operand_imm = (instruction & (1 << 25)) != 0;

    let set_carry =
        matches!(opcode, 0x0 | 0x1 | 0x8 | 0x9 | 0xC | 0xD | 0xE | 0xF) && set_condition_codes;

    let second_operand = if is_operand_imm {
        // Immediate operand (rotated right)
        let imm = instruction & 0xFF;
        let shift = (instruction & 0xF00) >> 7;
        cpu.rotr32(imm, shift, set_carry)
    } else {
        let rm = (instruction & 0xF) as i32;
        let mut value = cpu.get_register(rm);
        let shift_type = (instruction >> 5) & 0x3;

        let shift = if (instruction & (1 << 4)) != 0 {
            // Shift by register
            let rs = ((instruction >> 8) & 0xF) as i32;
            cpu.add_internal_cycles(1);

            if rm == (REG_PC as i32) {
                value = cpu.get_pc() + 4;
            }

            cpu.get_register(rs)
        } else {
            // Shift by immediate
            (instruction >> 7) & 0x1F
        } as i32;

        match shift_type {
            0 => cpu.lsl(value, shift, set_carry), // LSL
            1 => {
                if shift != 0 || (instruction & (1 << 4)) != 0 {
                    cpu.lsr(value, shift, set_carry)
                } else {
                    cpu.lsr_32(value, set_carry)
                }
            }
            2 => {
                if shift != 0 || (instruction & (1 << 4)) != 0 {
                    cpu.asr(value, shift, set_carry)
                } else {
                    cpu.asr_32(value, set_carry)
                }
            }
            3 => {
                if shift == 0 {
                    cpu.rrx(value, set_carry)
                } else {
                    cpu.rotr32(value, shift as u32, set_carry)
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
        0x0 => cpu.andd(
            destination as i32,
            first_operand_contents as i32,
            second_operand as i32,
            set_condition_codes,
        ),
        0x1 => cpu.eor(
            destination as i32,
            first_operand_contents as i32,
            second_operand as i32,
            set_condition_codes,
        ),
        0x2 => cpu.sub(
            destination,
            first_operand_contents,
            second_operand,
            set_condition_codes,
        ),
        0x3 => cpu.sub(
            destination,
            second_operand,
            first_operand_contents,
            set_condition_codes,
        ), // RSB
        0x4 => cpu.add(
            destination,
            first_operand_contents,
            second_operand,
            set_condition_codes,
        ),
        0x5 => cpu.adc(
            destination,
            first_operand_contents,
            second_operand,
            set_condition_codes,
        ),
        0x6 => cpu.sbc(
            destination,
            first_operand_contents,
            second_operand,
            set_condition_codes,
        ),
        0x7 => cpu.sbc(
            destination,
            second_operand,
            first_operand_contents,
            set_condition_codes,
        ), // RSC

        0x8 => {
            if set_condition_codes {
                cpu.tst(first_operand_contents, second_operand);
            } else {
                cpu.mrs(instruction);
            }
        }
        0x9 => {
            if set_condition_codes {
                cpu.teq(first_operand_contents, second_operand);
            } else {
                cpu.msr(instruction);
            }
        }
        0xA => {
            if set_condition_codes {
                cpu.cmp(first_operand_contents, second_operand);
            } else {
                cpu.mrs(instruction);
            }
        }
        0xB => {
            if set_condition_codes {
                cpu.cmn(first_operand_contents, second_operand);
            } else {
                cpu.msr(instruction);
            }
        }

        0xC => cpu.orr(
            destination as i32,
            first_operand_contents as i32,
            second_operand as i32,
            set_condition_codes,
        ),
        0xD => cpu.mov(destination, second_operand, set_condition_codes),
        0xE => cpu.bic(
            destination,
            first_operand_contents,
            second_operand,
            set_condition_codes,
        ),
        0xF => cpu.mvn(destination, second_operand, set_condition_codes),

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
pub fn count_leading_zeros(cpu: &mut ArmCpu, instruction: u32) {
    // CLZ is undefined when ID flag is set
    if cpu.get_id() <= 0 {
        #[cfg(feature = "tracing")]
        tracing::error!("CLZ executed while ID flag set (instr={instruction:#010X})");

        cpu.handle_undefined();
        return;
    }

    let source_reg = (instruction & 0xF) as i32;
    let destination = ((instruction >> 12) & 0xF) as i32;

    // if cpu.can_disassemble() { ... }

    let mut source = cpu.get_register(source_reg);

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

    cpu.set_register(destination, bits);
}

/// Saturated operation
pub fn saturated_op(cpu: &mut ArmCpu, instruction: u32) {
    // Saturated ops are undefined when ID flag is set
    if cpu.get_id() <= 0 {
        #[cfg(feature = "tracing")]
        tracing::error!("Saturated op executed while ID flag set (instr={instruction:#010X})");

        cpu.handle_undefined();
        return;
    }

    let opcode = (instruction >> 20) & 0xF;
    let operand = (instruction >> 16) & 0xF;
    let destination = (instruction >> 12) & 0xF;
    let source = (instruction & 0xF) as usize;

    let operand_reg = cpu.get_register(operand as i32);
    let source_reg = cpu.get_register(source as i32);

    let mut result: u32;

    match opcode {
        0x0 => {
            // QADD
            result = operand_reg.wrapping_add(source_reg);

            if add_overflow(source_reg, operand_reg, result) {
                cpu.get_cpsr_mut().sticky_overflow = true;

                result = if (result & 0x8000_0000) != 0 {
                    0x7FFF_FFFF
                } else {
                    0x8000_0000
                };
            }

            cpu.set_register(destination as i32, result);
        }

        0x2 => {
            // QSUB
            result = source_reg.wrapping_sub(operand_reg);

            if sub_overflow(source_reg, operand_reg, result) {
                cpu.get_cpsr_mut().sticky_overflow = true;

                result = if (result & 0x8000_0000) != 0 {
                    0x7FFF_FFFF
                } else {
                    0x8000_0000
                };
            }

            cpu.set_register(destination as i32, result);
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
pub fn multiply(cpu: &mut ArmCpu, instruction: u32) {
    let accumulate = instruction & (1 << 21);
    let set_condition_codes = instruction & (1 << 20);
    let destination = ((instruction >> 16) & 0xF) as i32;
    let first_operand = (instruction & 0xF) as i32;
    let second_operand = ((instruction >> 8) & 0xF) as i32;
    let third_operand = ((instruction >> 12) & 0xF) as i32;

    let mut result = cpu.get_register(first_operand) * cpu.get_register(second_operand);

    if accumulate != 0 {
        //if (cpu.can_disassemble())
        #[cfg(feature = "tracing")]
        tracing::error!("MLA {destination}, {first_operand}, {second_operand}, {third_operand}");
        result += cpu.get_register(third_operand);
    }
    //else if (cpu.can_disassemble())
    #[cfg(feature = "tracing")]
    tracing::error!("MUL {destination}, {first_operand}, {second_operand}");

    if set_condition_codes != 0 {
        cpu.set_zero_neg_flags(result);
    }

    cpu.set_register(destination, result);
}

/// Long multiply instruction
pub const fn multiply_long(cpu: &mut ArmCpu, instruction: u32) {
    let is_signed = (instruction & (1 << 22)) != 0;
    let accumulate = (instruction & (1 << 21)) != 0;
    let set_condition_codes = (instruction & (1 << 20)) != 0;

    let dest_hi = ((instruction >> 16) & 0xF) as i32;
    let dest_lo = ((instruction >> 12) & 0xF) as i32;
    let first_operand = ((instruction >> 8) & 0xF) as i32;
    let second_operand = (instruction & 0xF) as i32;

    let first_operand = cpu.get_register(first_operand);
    let second_operand = cpu.get_register(second_operand);

    if is_signed {
        // Signed long multiply
        let mut result = (first_operand as i32 as i64) * (second_operand as i32 as i64);

        if accumulate {
            let mut big_reg = cpu.get_register(dest_lo) as i64;
            big_reg |= (cpu.get_register(dest_hi) as i64) << 32;
            result = result.wrapping_add(big_reg);
        }

        cpu.set_register(dest_lo, result as u32);
        cpu.set_register(dest_hi, (result >> 32) as u32);

        if set_condition_codes {
            cpu.set_zero(result == 0);
            cpu.set_neg(((result >> 63) & 1) != 0);
        }
    } else {
        // Unsigned long multiply
        let mut result = (first_operand as u64) * (second_operand as u64);

        if accumulate {
            let mut big_reg = cpu.get_register(dest_lo) as u64;
            big_reg |= (cpu.get_register(dest_hi) as u64) << 32;
            result = result.wrapping_add(big_reg);
        }

        cpu.set_register(dest_lo, result as u32);
        cpu.set_register(dest_hi, (result >> 32) as u32);

        if set_condition_codes {
            cpu.set_zero(result == 0);
            cpu.set_neg(((result >> 63) & 1) != 0);
        }
    }
}

/// Signed halfword multiply instruction
pub fn signed_halfword_multiply(cpu: &mut ArmCpu, instruction: u32) {
    // No-op if ID bit set (matches C++)
    if cpu.get_id() > 0 {
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
            let op1 = cpu.get_register(first_operand);
            let op2 = cpu.get_register(second_operand);

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

            let acc = cpu.get_register(accumulate);
            let sum = (product as i64) + (acc as i64);
            result = sum as u32;

            if add_overflow(product as u32, acc, result) {
                cpu.get_cpsr_mut().sticky_overflow = true;
            }
        }

        0x9 => {
            // SMULWy / SMLAWy
            let op1 = cpu.get_register(first_operand);
            let op2 = cpu.get_register(second_operand);

            product = if first_op_top {
                (op1 >> 16) as i16 as i32
            } else {
                (op1 & 0xFFFF) as i16 as i32
            };

            let big_product = (product as i64 * op2 as i32 as i64) >> 16;

            if !second_op_top {
                // SMLAWy
                let acc = cpu.get_register(accumulate);
                let sum = big_product + acc as i64;
                result = sum as u32;

                if add_overflow(big_product as u32, acc, result) {
                    cpu.get_cpsr_mut().sticky_overflow = true;
                }
            } else {
                // SMULWy
                result = big_product as u32;
            }
        }

        0xB => {
            // SMULxy
            let op1 = cpu.get_register(first_operand);
            let op2 = cpu.get_register(second_operand);

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

    cpu.set_register(destination, result);
}

/// Swap instruction
pub fn swap(cpu: &mut ArmCpu, instruction: u32) {
    let is_byte = (instruction & (1 << 22)) != 0;
    let base = ((instruction >> 16) & 0xF) as i32;
    let destination = ((instruction >> 12) & 0xF) as i32;
    let source = (instruction & 0xF) as i32;

    let address = cpu.get_register(base);

    if is_byte {
        // SWPB
        // let byte = cpu.read_byte(address);
        let byte = 0;
        // cpu.write_byte(address, (cpu.get_register(source) & 0xFF) as u8);
        cpu.set_register(destination, byte as u32);

        cpu.add_n16_data(address, 2);
    } else {
        // SWP (word)
        let aligned = address & !0x3;
        let rotate = (address & 0x3) * 8;

        let word = cpu.rotr32(
            // cpu.read_word(aligned),
            0, rotate, false,
        );

        // cpu.write_word(address, cpu.get_register(source));
        cpu.set_register(destination, word);

        cpu.add_n32_data(address, 2);
    }
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
    // // let cp15 = cpu.get_cp15();
    // if cp15.is_none() {
    //     // Coprocessor not present → instruction ignored
    //     return;
    // }
    // let cp15 = cp15.unwrap();

    // let operation_mode = ((instruction >> 21) & 0x7) as u32;
    // let is_loading = (instruction & (1 << 20)) != 0;

    // let cp_reg = ((instruction >> 16) & 0xF) as u32;
    // let arm_reg = ((instruction >> 12) & 0xF) as usize;

    // let coprocessor_id = ((instruction >> 8) & 0xF) as u32;
    // let coprocessor_info = ((instruction >> 5) & 0x7) as u32;
    // let coprocessor_operand = (instruction & 0xF) as u32;

    // if is_loading {
    //     // MRC
    //     cpu.add_internal_cycles(2);
    //     cpu.add_cop_cycles(1);

    //     match coprocessor_id {
    //         15 => {
    //             let value = cp15.mrc(
    //                 operation_mode,
    //                 cp_reg,
    //                 coprocessor_info,
    //                 coprocessor_operand,
    //             );
    //             cpu.set_register(arm_reg, value);
    //         }
    //         _ => {
    //             // mirrors printf + exit(1)
    //             panic!("Coprocessor {} not recognized", coprocessor_id);
    //         }
    //     }
    // } else {
    //     // MCR
    //     cpu.add_internal_cycles(1);
    //     cpu.add_cop_cycles(1);

    //     match coprocessor_id {
    //         15 => {
    //             let value = cpu.get_register(arm_reg);
    //             cp15.mcr(
    //                 operation_mode,
    //                 cp_reg,
    //                 value,
    //                 coprocessor_info,
    //                 coprocessor_operand,
    //             );
    //         }
    //         _ => {
    //             panic!("Coprocessor {} not recognized", coprocessor_id);
    //         }
    //     }
    // }
}

/// Branch with link and exchange (register)
pub fn blx_reg(cpu: &mut ArmCpu, instruction: u32) {
    cpu.set_register(REG_LR as i32, cpu.get_pc() - 4);

    let reg_id = (instruction & 0xF) as i32;

    //if (cpu.can_disassemble())
    //printf("BLX {%d}", reg_id);

    let new_address = cpu.get_register(reg_id);
}

/// Branch with link and exchange (immediate)
pub fn blx(cpu: &mut ArmCpu, instruction: u32) {
    todo!()
}

/// Software interrupt
pub fn swi(cpu: &mut ArmCpu, instruction: u32) {
    cpu.handle_swi();
}
