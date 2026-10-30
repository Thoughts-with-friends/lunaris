use crate::{arm_cpu::ArmCpu, instruction_table::ARMInstr};

/// Disassembles one ARM instruction (debug log of ds_emu arm_interpret).
pub fn disasm_arm(cpu: &mut ArmCpu, instruction: u32, address: u32) -> String {
    let opcode = arm_decode(instruction);

    match opcode {
        ARMInstr::DataProcessing => disasm_data_processing(cpu, instruction),

        ARMInstr::StoreByte | ARMInstr::LoadByte | ARMInstr::StoreWord | ARMInstr::LoadWord => {
            disasm_load_store(cpu, instruction)
        }

        ARMInstr::Branch | ARMInstr::BranchWithLink => disasm_branch(cpu, instruction, address),

        ARMInstr::BranchExchange => disasm_bx(instruction),

        _ => "(UNDEFINED)".to_string(),
    }
}

/// Data processing (AND..MVN) text.
fn disasm_data_processing(cpu: &mut ArmCpu, instruction: u32) -> String {
    // ARM Opcodes: Data Processing (ALU): https://problemkaputt.de/gbatek.htm#armopcodesdataprocessingalu
    let opcode = (instruction >> 21) & 0xF; // ALU opcode (bits 21-24)
    let first_operand = (instruction >> 16) & 0xF; // Rn (bits 16-19)
    let destination = (instruction >> 12) & 0xF; // Rd (bits 12-15)

    let set_cc = (instruction & (1 << 20)) != 0; // S bit (20): set condition codes
    let is_imm = (instruction & (1 << 25)) != 0; // I bit (25): immediate operand

    let second_operand_str = if is_imm {
        let mut operand = instruction & 0xFF; // 8-bit immediate (bits 0-7)
        let shift = (instruction & 0xF00) >> 7; // rotate field (bits 8-11) * 2
        operand = cpu.rotr32(operand, shift, false);

        format!("#0x{operand:X}")
    } else {
        let reg_name = ArmCpu::get_reg_name(instruction & 0xF); // Rm (bits 0-3)

        let shift_type = (instruction >> 5) & 0x3; // shift type (bits 5-6)
        let shift_name = match shift_type {
            0 => "lsl",
            1 => "lsr",
            2 => "asr",
            _ => "ror",
        };

        let src = if (instruction & (1 << 4)) != 0 {
            // bit 4: shift by register
            let rs = (instruction >> 8) & 0xF; // Rs (bits 8-11)
            format!(" {}", ArmCpu::get_reg_name(rs))
        } else {
            let imm = (instruction >> 7) & 0x1F; // immediate shift amount (bits 7-11)
            format!(" #{imm}")
        };

        format!("{reg_name}, {shift_name}{src}")
    };

    let mut mnemonic = match opcode {
        0x0 => "and", // AND
        0x1 => "eor", // EOR
        0x2 => "sub", // SUB
        0x3 => "rsb", // RSB
        0x4 => "add", // ADD
        0x5 => "adc", // ADC
        0x6 => "sbc", // SBC
        0x7 => "rsc", // RSC

        0x8 => {
            // TST
            if set_cc {
                "tst"
            } else {
                return disasm_mrs(cpu, instruction);
            }
        }

        0x9 => {
            // TEQ
            if set_cc {
                "teq"
            } else {
                return disasm_msr(cpu, instruction);
            }
        }

        0xA => {
            // CMP
            if set_cc {
                "cmp"
            } else {
                return disasm_mrs(cpu, instruction);
            }
        }

        0xB => {
            // CMN
            if set_cc {
                "cmn"
            } else {
                return disasm_msr(cpu, instruction);
            }
        }

        0xC => "orr", // ORR
        0xD => "mov", // MOV
        0xE => "bic", // BIC
        _ => "mvn",
    }
    .to_string();

    if set_cc {
        mnemonic.push('s');
    }

    mnemonic.push_str(ArmCpu::get_condition_name(instruction >> 28));

    format!(
        "{} {}, {}, {}",
        mnemonic,
        ArmCpu::get_reg_name(destination),
        ArmCpu::get_reg_name(first_operand),
        second_operand_str
    )
}

/// MRS text.
fn disasm_mrs(_cpu: &ArmCpu, instruction: u32) -> String {
    let cond = ArmCpu::get_condition_name(instruction >> 28);
    // ARM Opcodes: PSR Transfer (MRS, MSR): https://problemkaputt.de/gbatek.htm#armopcodespsrtransfermrsmsr
    let rd = ArmCpu::get_reg_name((instruction >> 12) & 0xF); // Rd (bits 12-15)

    let using_cpsr = (instruction & (1 << 22)) == 0; // Psr bit (22): 0 = CPSR, 1 = SPSR
    let psr = if using_cpsr { "CPSR" } else { "SPSR" };

    format!("mrs{} {}, {}", cond, rd, psr)
}

/// MSR text.
fn disasm_msr(cpu: &mut ArmCpu, instruction: u32) -> String {
    let cond = ArmCpu::get_condition_name(instruction >> 28);

    let using_cpsr = (instruction & (1 << 22)) == 0; // Psr bit (22): 0 = CPSR, 1 = SPSR
    let psr = if using_cpsr { "CPSR" } else { "SPSR" };

    let is_imm = (instruction & (1 << 25)) != 0; // I bit (25): immediate operand

    let operand_str = if is_imm {
        // ARM Opcodes: PSR Transfer (MRS, MSR): https://problemkaputt.de/gbatek.htm#armopcodespsrtransfermrsmsr
        let mut op = instruction & 0xFF; // 8-bit immediate (bits 0-7)
        let shift = (instruction & 0xFF0) >> 7; // rotate field (bits 8-11) * 2
        op = cpu.rotr32(op, shift, false);

        format!("#0x{:X}", op)
    } else {
        ArmCpu::get_reg_name(instruction & 0xF).to_string() // Rm (bits 0-3)
    };

    format!("msr{} {}, {}", cond, psr, operand_str)
}

/// LDR/STR text.
fn disasm_load_store(_cpu: &ArmCpu, instruction: u32) -> String {
    let mut out = String::new();

    if (instruction & (1 << 20)) != 0 {
        // L bit (20): load
        out.push_str("ldr");
    } else {
        out.push_str("str");
    }

    if (instruction & (1 << 22)) != 0 {
        // B bit (22): byte transfer
        out.push('b');
    }

    out.push_str(ArmCpu::get_condition_name(instruction >> 28));

    let is_imm = (instruction & (1 << 25)) == 0; // I bit (25): 0 = immediate offset

    let offset_str = if is_imm {
        // ARM Opcodes: Memory: Single Data Transfer (LDR, STR, PLD): https://problemkaputt.de/gbatek.htm#armopcodesmemorysingledatatransferldrstrpld
        format!("#0x{:X}", instruction & 0xFFF) // 12-bit immediate offset (bits 0-11)
    } else {
        "???".to_string()
    };

    let pre_index = (instruction & (1 << 24)) != 0; // P bit (24): pre-indexing

    let reg = (instruction >> 12) & 0xF; // Rd (bits 12-15)
    let base = (instruction >> 16) & 0xF; // Rn (bits 16-19)

    out.push_str(&format!(" {}, [{}", ArmCpu::get_reg_name(reg), ArmCpu::get_reg_name(base)));

    if pre_index {
        out.push_str(&format!(", {}", offset_str));
    }

    out.push(']');

    if (instruction & (1 << 21)) != 0 {
        // W bit (21): write-back
        out.push('!');
    }

    if !pre_index {
        out.push_str(&format!(", {}", offset_str));
    }

    out
}

/// B/BL text with the absolute target.
fn disasm_branch(cpu: &ArmCpu, instruction: u32, address: u32) -> String {
    let condition = instruction >> 28;

    // ARM Opcodes: Branch and Branch with Link (B, BL, BX, BLX, SWI, BKPT): https://problemkaputt.de/gbatek.htm#armopcodesbranchandbranchwithlinkbblbxblxswibkpt
    let mut offset = ((instruction & 0xFFFFFF) << 2) as i32; // 24-bit signed word offset (bits 0-23) * 4
    offset <<= 6;
    offset >>= 6;

    if condition == 15 {
        if cpu.get_id() != 0 {
            return "UNDEFINED".into();
        } else {
            if (instruction & (1 << 24)) != 0 {
                // L bit (24): branch with link
                offset += 2;
            }

            return format!("blx $0x{:X}", address.wrapping_add(offset as u32));
        }
    }

    let mut out = String::from("b");

    if (instruction & (1 << 24)) != 0 {
        // L bit (24): branch with link
        out.push('l');
    }

    out.push_str(ArmCpu::get_condition_name(condition));

    out.push_str(&format!(" $0x{:X}", address.wrapping_add(offset as u32)));

    out
}

/// BX text.
pub fn disasm_bx(instruction: u32) -> String {
    format!(
        "bx{} {}",
        ArmCpu::get_condition_name(instruction >> 28),
        ArmCpu::get_reg_name(instruction & 0xF) // Rm (bits 0-3)
    )
}
/// Decodes an ARM instruction into an enum
pub const fn arm_decode(instruction: u32) -> ARMInstr {
    // Here be dragons

    // Branch
    // ARM Instruction Summary: https://problemkaputt.de/gbatek.htm#arminstructionsummary
    if ((instruction & 0x0F00_0000) >> 24) == 0xA {
        // bits 24-27 = 1010b: B
        return ARMInstr::Branch;
    }

    // Branch with link
    if ((instruction & 0x0F00_0000) >> 24) == 0xB {
        // bits 24-27 = 1011b: BL
        return ARMInstr::BranchWithLink;
    }

    // BX
    if ((instruction >> 4) & 0x00FF_FFFF) == 0x12FFF1 {
        // BX Rn: cond 0001 0010 1111 1111 1111 0001 Rn
        return ARMInstr::BranchExchange;
    }

    // BLX
    if ((instruction >> 4) & 0x00FF_FFFF) == 0x12FFF3 {
        // BLX Rn: cond 0001 0010 1111 1111 1111 0011 Rn
        return ARMInstr::BranchLinkExchange;
    }

    // ARMv5TE-only instructions
    if ((instruction >> 16) & 0xFFF) == 0x16F && ((instruction >> 4) & 0xFF) == 0xF1 {
        // CLZ: bits 16-27 = 16Fh, bits 4-11 = F1h
        return ARMInstr::CountLeadingZeros;
    }

    // Saturated ops
    if ((instruction >> 4) & 0xFF) == 0x05 && ((instruction >> 24) & 0xF) == 0x1 {
        // QADD/QSUB/QDADD/QDSUB: bits 4-7 = 0101b, bits 24-27 = 0001b
        let op = (instruction >> 20) & 0xF; // opcode (bits 20-23)
        if matches!(op, 0 | 2 | 4 | 6) {
            return ARMInstr::SaturatedOp;
        }
    }

    // Data processing / multiply / halfword hell
    if ((instruction >> 26) & 0x3) == 0 {
        // bits 26-27 = 00b: data processing / multiply / misc
        if (instruction & (1 << 25)) == 0 {
            // SWP
            if ((instruction >> 4) & 0xFF) == 0x9 // SWP/SWPB: bits 4-7 = 1001b
                && ((instruction >> 23) & 0x1F) == 0x2 // bits 23-27 = 00010b
                && ((instruction >> 20) & 0x3) == 0
            // bits 20-21 = 00b
            {
                return ARMInstr::Swap;
            }

            // Signed halfword multiply
            if (instruction & (1 << 7)) != 0 // bit 7 set
                && (instruction & (1 << 4)) == 0 // bit 4 clear
                && (instruction & (1 << 20)) == 0 // bit 20 clear: SMLAxy family
                && ((instruction >> 23) & 0x3) == 0x2
            // bits 23-24 = 10b
            {
                return ARMInstr::SignedHalfwordMultiply;
            }

            // Multiply / halfword transfers
            if (instruction & (1 << 7)) != 0 && (instruction & (1 << 4)) != 0 {
                // bit 4: shift by register
                if ((instruction >> 4) & 0xF) == 0x9 {
                    // bits 4-7 = 1001b: multiply
                    if ((instruction >> 22) & 0x3F) == 0 {
                        // bits 22-27 = 0: MUL/MLA
                        return ARMInstr::Multiply;
                    } else if ((instruction >> 23) & 0x1F) == 1 {
                        // bits 23-27 = 00001b: UMULL/UMLAL/SMULL/SMLAL
                        return ARMInstr::MultiplyLong;
                    }
                    return ARMInstr::Undefined;
                } else if (instruction & (1 << 6)) == 0 && (instruction & (1 << 5)) != 0 {
                    // SH = 01b: unsigned halfword
                    if (instruction & (1 << 20)) != 0 {
                        // L bit (20): load
                        return ARMInstr::LoadHalfword;
                    } else {
                        return ARMInstr::StoreHalfword;
                    }
                } else if (instruction & (1 << 6)) != 0 && (instruction & (1 << 5)) == 0 {
                    // SH = 10b: signed byte
                    if (instruction & (1 << 20)) != 0 {
                        // L bit (20): load
                        return ARMInstr::LoadSignedByte;
                    } else {
                        return ARMInstr::LoadDoubleword;
                    }
                } else if (instruction & (1 << 6)) != 0 && (instruction & (1 << 5)) != 0 {
                    // SH = 11b: signed halfword
                    if (instruction & (1 << 20)) != 0 {
                        // L bit (20): load
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
        // bits 26-27 = 01b: LDR/STR
        if (instruction & (1 << 20)) == 0 {
            // L bit (20): 0 = store
            if (instruction & (1 << 22)) == 0 {
                // B bit (22): 0 = word
                return ARMInstr::StoreWord;
            } else {
                return ARMInstr::StoreByte;
            }
        } else if (instruction & (1 << 22)) == 0 {
            // B bit (22): 0 = word
            return ARMInstr::LoadWord;
        } else {
            return ARMInstr::LoadByte;
        }
    }

    // Block transfer
    if ((instruction >> 25) & 0x7) == 0x4 {
        // bits 25-27 = 100b: LDM/STM
        if (instruction & (1 << 20)) == 0 {
            // L bit (20): 0 = store
            return ARMInstr::StoreBlock;
        } else {
            return ARMInstr::LoadBlock;
        }
    }

    // Coprocessor
    if ((instruction >> 24) & 0xF) == 0xE {
        // bits 24-27 = 1110b: coprocessor
        if (instruction & (1 << 4)) != 0 {
            // bit 4: shift by register
            return ARMInstr::CopRegTransfer;
        } else {
            return ARMInstr::CopDataOp;
        }
    }

    // SWI
    if ((instruction >> 24) & 0xF) == 0xF {
        // bits 24-27 = 1111b: SWI
        return ARMInstr::Swi;
    }

    ARMInstr::Undefined
}
