// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
use crate::cpu::arm_cpu::CpuType;
use crate::emulator::Emulator;

impl Emulator {
    /// Retrieves the SWI opcode using the ARM CPU's LR logic.
    /// The handler compensates based on ARM vs THUMB mode.
    fn get_opcode(&self, cpu_type: CpuType) -> u8 {
        let mut lr = self.get_cpu(cpu_type).get_pc();
        if self.get_cpu(cpu_type).get_cpsr().thumb_on {
            lr -= 2;
        } else {
            lr -= 4;
        }

        (self.read_halfword(lr - 2, cpu_type) & 0xFF) as u8
    }

    /// Implements the BIOS signed division routine.
    /// r0 = quotient, r1 = remainder, r3 = |quotient|
    #[expect(unused)]
    fn div(&mut self, cpu_type: CpuType) {
        let dividend = self.get_cpu(cpu_type).get_register(0) as i32;
        let divisor = self.get_cpu(cpu_type).get_register(1) as i32;

        let quotient = dividend / divisor;

        self.get_cpu_mut(cpu_type).set_register(0, quotient as u32);
        self.get_cpu_mut(cpu_type)
            .set_register(1, (dividend % divisor) as u32);
        self.get_cpu_mut(cpu_type)
            .set_register(3, quotient.unsigned_abs());
    }

    /// Implements BIOS CpuSet memory transfer/fill.
    /// Behaviour follows ARM9/ARM7 NDS semantics.
    fn cpu_set(&mut self, cpu_type: CpuType) {
        let mut source = self.get_cpu_mut(cpu_type).get_register(0);
        let mut dest = self.get_cpu_mut(cpu_type).get_register(1);
        let flags = self.get_cpu_mut(cpu_type).get_register(2);

        if self.get_cpu_mut(cpu_type).get_id() != 0 && source < 0x4000 && dest < 0x4000 {
            return;
        }

        let mut len = flags & 0x1FFFFF;
        let memfill = flags & (1 << 24) != 0;
        let size_word = flags & (1 << 26) != 0;

        while len != 0 {
            if size_word {
                let v = self.read_word(source, cpu_type);
                self.write_word(dest, v, cpu_type);
            } else {
                let v = self.read_halfword(source, cpu_type);
                self.write_halfword(dest, v, cpu_type);
            }

            if !memfill {
                source += 2 << (size_word as u32);
            }
            dest += 2 << (size_word as u32);

            len -= 1;
        }
    }

    /// Approximates `SWI 0x04` (`IntrWait`) / `SWI 0x05` (`VBlankIntrWait`).
    ///
    /// The real BIOS routine waits for a *new* occurrence of one of the
    /// interrupts in `wait_mask`, tracked via a caller-maintained shared RAM
    /// flag distinct from IE/IF, and only returns once the BIOS's own IRQ
    /// handler observes and acknowledges it. This emulator has no HLE IRQ
    /// dispatcher to maintain that shared flag, so instead this enables the
    /// requested interrupt bit(s) in IE, enables IME, and halts the CPU —
    /// relying on the existing hardware interrupt-request path (already
    /// used by `SWI 0x06` `Halt`) to wake it. This is not bit-exact (it
    /// does not honor "discard stale flags" and leaves the bits enabled in
    /// IE afterward) but is enough to unblock games whose main loop
    /// synchronizes every frame via `SWI 0x05`.
    fn intr_wait(&mut self, cpu_type: CpuType, wait_mask: u32) {
        if self.get_cpu(cpu_type).get_id() == 0 {
            self.int9_reg.irq_enable |= wait_mask;
            self.int9_reg.ime = 1;
        } else {
            self.int7_reg.irq_enable |= wait_mask;
            self.int7_reg.ime = 1;
        }
        self.get_cpu_mut(cpu_type).halt();
    }

    /// Calculates CRC16 over a sequence of halfwords.
    fn get_crc16(&mut self, cpu_type: CpuType) {
        let crcs: [u16; 8] = [
            0xC0C1, 0xC181, 0xC301, 0xC601, 0xCC01, 0xD801, 0xF001, 0xA001,
        ];

        let mut crc = (self.get_cpu_mut(cpu_type).get_register(0) & 0xFFFF) as u16;
        let mut addr = self.get_cpu_mut(cpu_type).get_register(1);
        let len = self.get_cpu_mut(cpu_type).get_register(2);
        let end = addr + len;

        while addr < end {
            crc ^= self.read_halfword(addr, cpu_type);

            for (i, crc_) in crcs.iter().enumerate() {
                let carry = (crc & 1) != 0;
                crc >>= 1;
                if carry {
                    crc ^= crc_ << (7 - i);
                }
            }

            addr += 2;
        }

        self.get_cpu_mut(cpu_type).set_register(0, crc as u32);
    }

    /// Executes SWI 7 (ARM7 BIOS).
    pub fn swi7(&mut self) -> i32 {
        let opcode = self.get_opcode(CpuType::Arm7);
        match opcode {
            0x03 => {
                let reg = self.arm7.get_register(0);
                self.arm7.add_internal_cycles((reg * 4) as i32);
            }
            0x04 => {
                let wait_mask = self.arm7.get_register(1);
                self.intr_wait(CpuType::Arm7, wait_mask);
            }
            0x05 => self.intr_wait(CpuType::Arm7, 1), // VBlank (IE/IF bit 0)
            0x06 => self.arm7.halt(),
            0x0B => self.cpu_set(CpuType::Arm7),
            0x0E => self.get_crc16(CpuType::Arm7),
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Unrecognized HLE SWI7 ${opcode:02X}");
            }
        }
        1
    }

    /// Executes SWI 9 (ARM9 BIOS).
    pub fn swi9(&mut self) -> i32 {
        let opcode = self.get_opcode(CpuType::Arm9);
        match opcode {
            0x03 => {
                let value = self.get_cpu(CpuType::Arm9).get_register(0) * 2;
                self.get_cpu_mut(CpuType::Arm9)
                    .add_internal_cycles(value as i32);
            }
            0x04 => {
                let wait_mask = self.get_cpu(CpuType::Arm9).get_register(1);
                self.intr_wait(CpuType::Arm9, wait_mask);
            }
            0x05 => self.intr_wait(CpuType::Arm9, 1), // VBlank (IE/IF bit 0)
            0x06 => self.get_cpu_mut(CpuType::Arm9).halt(),
            0x0B => self.cpu_set(CpuType::Arm9),
            0x0E => self.get_crc16(CpuType::Arm9),
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Unrecognized HLE SWI9 ${opcode:02X}");
            }
        }
        1
    }
}
