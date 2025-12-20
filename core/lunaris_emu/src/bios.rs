// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
use crate::cpu::ARMCPU;

/// BIOS emulation for CorgiDS.
pub struct BIOS;

impl BIOS {
    /// Creates a new BIOS instance.
    pub fn new() -> Self {
        BIOS
    }

    /// Retrieves the SWI opcode using the ARM CPU's LR logic.
    /// The handler compensates based on ARM vs THUMB mode.
    fn get_opcode(&self, cpu: &mut ARMCPU) -> u8 {
        let mut lr = cpu.get_pc();
        if cpu.get_cpsr().thumb_on {
            lr -= 2;
        } else {
            lr -= 4;
        }

        (cpu.read_halfword(lr - 2) & 0xFF) as u8
    }

    /// Implements the BIOS signed division routine.
    /// r0 = quotient, r1 = remainder, r3 = |quotient|
    fn div(&self, cpu: &mut ARMCPU) {
        let dividend = cpu.get_register(0) as i32;
        let divisor = cpu.get_register(1) as i32;

        let quotient = dividend / divisor;

        cpu.set_register(0, quotient as u32);
        cpu.set_register(1, (dividend % divisor) as u32);
        cpu.set_register(3, quotient.abs() as u32);
    }

    /// Implements BIOS CpuSet memory transfer/fill.
    /// Behaviour follows ARM9/ARM7 NDS semantics.
    fn cpu_set(&self, cpu: &mut ARMCPU) {
        let mut source = cpu.get_register(0);
        let mut dest = cpu.get_register(1);
        let flags = cpu.get_register(2);

        if cpu.get_id() != 0 {
            if source < 0x4000 && dest < 0x4000 {
                return;
            }
        }

        let mut len = flags & 0x1FFFFF;
        let memfill = flags & (1 << 24) != 0;
        let size_word = flags & (1 << 26) != 0;

        while len != 0 {
            if size_word {
                let v = cpu.read_word(source);
                cpu.write_word(dest, v);
            } else {
                let v = cpu.read_halfword(source);
                cpu.write_halfword(dest, v);
            }

            if !memfill {
                source += 2 << (size_word as u32);
            }
            dest += 2 << (size_word as u32);

            len -= 1;
        }
    }

    /// Calculates CRC16 over a sequence of halfwords.
    fn get_crc16(&self, cpu: &mut ARMCPU) {
        let crcs: [u16; 8] = [
            0xC0C1, 0xC181, 0xC301, 0xC601, 0xCC01, 0xD801, 0xF001, 0xA001,
        ];

        let mut crc = (cpu.get_register(0) & 0xFFFF) as u16;
        let mut addr = cpu.get_register(1);
        let len = cpu.get_register(2);
        let end = addr + len;

        while addr < end {
            crc ^= cpu.read_halfword(addr) as u16;

            for i in 0..8 {
                let carry = (crc & 1) != 0;
                crc >>= 1;
                if carry {
                    crc ^= crcs[i] << (7 - i);
                }
            }

            addr += 2;
        }

        cpu.set_register(0, crc as u32);
    }

    /// Executes SWI 7 (ARM7 BIOS).
    pub fn swi7(&self, arm7: &mut ARMCPU) -> i32 {
        let opcode = self.get_opcode(arm7);
        match opcode {
            0x03 => arm7.add_internal_cycles((arm7.get_register(0) * 4) as i32),
            0x06 => arm7.halt(),
            0x0B => self.cpu_set(arm7),
            0x0E => self.get_crc16(arm7),
            _ => {
                eprintln!("Unrecognized HLE SWI7 ${:02X}", opcode);
                std::process::exit(1);
            }
        }
        1
    }

    /// Executes SWI 9 (ARM9 BIOS).
    pub fn swi9(&self, arm9: &mut ARMCPU) -> i32 {
        let opcode = self.get_opcode(arm9);
        match opcode {
            0x03 => arm9.add_internal_cycles((arm9.get_register(0) * 2) as i32),
            0x06 => arm9.halt(),
            0x0B => self.cpu_set(arm9),
            0x0E => self.get_crc16(arm9),
            _ => {
                eprintln!("Unrecognized HLE SWI9 ${:02X}", opcode);
                std::process::exit(1);
            }
        }
        1
    }
}
