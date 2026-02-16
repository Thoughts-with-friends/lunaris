use crate::cpu::arm_cpu::ArmCpu;
use crate::cpu::instruction_table::ARMInstr;

pub fn disasm_arm(cpu: &mut ArmCpu, instruction: u32, address: u32) -> String {
    let opcode = super::interpreter::arm_decode(instruction);

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

fn disasm_data_processing(cpu: &mut ArmCpu, instruction: u32) -> String {
    let opcode = (instruction >> 21) & 0xF;
    let first_operand = (instruction >> 16) & 0xF;
    let destination = (instruction >> 12) & 0xF;

    let set_cc = (instruction & (1 << 20)) != 0;
    let is_imm = (instruction & (1 << 25)) != 0;

    let second_operand_str = if is_imm {
        let mut operand = instruction & 0xFF;
        let shift = (instruction & 0xF00) >> 7;
        operand = cpu.rotr32(operand, shift, false);

        format!("#0x{operand:X}")
    } else {
        let reg_name = ArmCpu::get_reg_name(instruction & 0xF);

        let shift_type = (instruction >> 5) & 0x3;
        let shift_name = match shift_type {
            0 => "lsl",
            1 => "lsr",
            2 => "asr",
            _ => "ror",
        };

        let src = if (instruction & (1 << 4)) != 0 {
            let rs = (instruction >> 8) & 0xF;
            format!(" {}", ArmCpu::get_reg_name(rs))
        } else {
            let imm = (instruction >> 7) & 0x1F;
            format!(" #{imm}")
        };

        format!("{reg_name}, {shift_name}{src}")
    };

    let mut mnemonic = match opcode {
        0x0 => "and",
        0x1 => "eor",
        0x2 => "sub",
        0x3 => "rsb",
        0x4 => "add",
        0x5 => "adc",
        0x6 => "sbc",
        0x7 => "rsc",

        0x8 => {
            if set_cc {
                "tst"
            } else {
                return disasm_mrs(cpu, instruction);
            }
        }

        0x9 => {
            if set_cc {
                "teq"
            } else {
                return disasm_msr(cpu, instruction);
            }
        }

        0xA => {
            if set_cc {
                "cmp"
            } else {
                return disasm_mrs(cpu, instruction);
            }
        }

        0xB => {
            if set_cc {
                "cmn"
            } else {
                return disasm_msr(cpu, instruction);
            }
        }

        0xC => "orr",
        0xD => "mov",
        0xE => "bic",
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

fn disasm_mrs(_cpu: &ArmCpu, instruction: u32) -> String {
    let cond = ArmCpu::get_condition_name(instruction >> 28);
    let rd = ArmCpu::get_reg_name((instruction >> 12) & 0xF);

    let using_cpsr = (instruction & (1 << 22)) == 0;
    let psr = if using_cpsr { "CPSR" } else { "SPSR" };

    format!("mrs{} {}, {}", cond, rd, psr)
}

fn disasm_msr(cpu: &mut ArmCpu, instruction: u32) -> String {
    let cond = ArmCpu::get_condition_name(instruction >> 28);

    let using_cpsr = (instruction & (1 << 22)) == 0;
    let psr = if using_cpsr { "CPSR" } else { "SPSR" };

    let is_imm = (instruction & (1 << 25)) != 0;

    let operand_str = if is_imm {
        let mut op = instruction & 0xFF;
        let shift = (instruction & 0xFF0) >> 7;
        op = cpu.rotr32(op, shift, false);

        format!("#0x{:X}", op)
    } else {
        ArmCpu::get_reg_name(instruction & 0xF).to_string()
    };

    format!("msr{} {}, {}", cond, psr, operand_str)
}

fn disasm_load_store(_cpu: &ArmCpu, instruction: u32) -> String {
    let mut out = String::new();

    if (instruction & (1 << 20)) != 0 {
        out.push_str("ldr");
    } else {
        out.push_str("str");
    }

    if (instruction & (1 << 22)) != 0 {
        out.push('b');
    }

    out.push_str(ArmCpu::get_condition_name(instruction >> 28));

    let is_imm = (instruction & (1 << 25)) == 0;

    let offset_str = if is_imm {
        format!("#0x{:X}", instruction & 0xFFF)
    } else {
        "???".to_string()
    };

    let pre_index = (instruction & (1 << 24)) != 0;

    let reg = (instruction >> 12) & 0xF;
    let base = (instruction >> 16) & 0xF;

    out.push_str(&format!(
        " {}, [{}",
        ArmCpu::get_reg_name(reg),
        ArmCpu::get_reg_name(base)
    ));

    if pre_index {
        out.push_str(&format!(", {}", offset_str));
    }

    out.push(']');

    if (instruction & (1 << 21)) != 0 {
        out.push('!');
    }

    if !pre_index {
        out.push_str(&format!(", {}", offset_str));
    }

    out
}

fn disasm_branch(cpu: &ArmCpu, instruction: u32, address: u32) -> String {
    let condition = instruction >> 28;

    let mut offset = ((instruction & 0xFFFFFF) << 2) as i32;
    offset <<= 6;
    offset >>= 6;

    if condition == 15 {
        if cpu.get_id() != 0 {
            return "UNDEFINED".into();
        } else {
            if (instruction & (1 << 24)) != 0 {
                offset += 2;
            }

            return format!("blx $0x{:X}", address.wrapping_add(offset as u32));
        }
    }

    let mut out = String::from("b");

    if (instruction & (1 << 24)) != 0 {
        out.push('l');
    }

    out.push_str(ArmCpu::get_condition_name(condition));

    out.push_str(&format!(" $0x{:X}", address.wrapping_add(offset as u32)));

    out
}

pub fn disasm_bx(instruction: u32) -> String {
    format!(
        "bx{} {}",
        ArmCpu::get_condition_name(instruction >> 28),
        ArmCpu::get_reg_name(instruction & 0xF)
    )
}
