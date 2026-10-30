// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! Connects the high-level BIOS ([`ds_bios::swi`]) to the CPU that executed
//! `SWI`.
use ds_bios::swi::{self, SwiContext};
use ds_cpu::arm_cpu::CpuType;

use crate::emulator::Emulator;

/// The calling CPU and the bus it sees.
struct Caller<'a> {
    emu: &'a mut Emulator,
    cpu: CpuType,
}

impl SwiContext for Caller<'_> {
    fn reg(&self, index: usize) -> u32 {
        self.emu.get_cpu(self.cpu).get_register(index as _)
    }

    fn set_reg(&mut self, index: usize, value: u32) {
        self.emu.get_cpu_mut(self.cpu).set_register(index as _, value);
    }

    fn pc(&self) -> u32 {
        self.emu.get_cpu(self.cpu).get_pc()
    }

    fn is_thumb(&self) -> bool {
        self.emu.get_cpu(self.cpu).get_cpsr().thumb_on
    }

    fn read8(&mut self, addr: u32) -> u8 {
        self.emu.read_byte(addr, self.cpu)
    }

    fn read16(&mut self, addr: u32) -> u16 {
        self.emu.read_halfword(addr, self.cpu)
    }

    fn read32(&mut self, addr: u32) -> u32 {
        self.emu.read_word(addr, self.cpu)
    }

    fn write16(&mut self, addr: u32, value: u16) {
        self.emu.write_halfword(addr, value, self.cpu);
    }

    fn write32(&mut self, addr: u32, value: u32) {
        self.emu.write_word(addr, value, self.cpu);
    }

    fn add_cycles(&mut self, cycles: i32) {
        self.emu.get_cpu_mut(self.cpu).add_internal_cycles(cycles);
    }

    fn halt(&mut self) {
        self.emu.get_cpu_mut(self.cpu).halt();
    }

    /// Approximates `IntrWait` / `VBlankIntrWait`: the real BIOS waits for
    /// a *new* occurrence tracked in a shared RAM flag; this enables the
    /// interrupts in IE and IME and halts, relying on the normal interrupt
    /// path to wake the CPU (enough for games that sync on VBlank).
    fn wait_for_interrupt(&mut self, mask: u32) {
        let int = match self.cpu {
            CpuType::Arm9 => &mut self.emu.int9_reg,
            CpuType::Arm7 => &mut self.emu.int7_reg,
        };
        int.irq_enable |= mask;
        int.ime = 1;
        self.emu.get_cpu_mut(self.cpu).halt();
    }
}

impl Emulator {
    /// Runs the BIOS function requested by `SWI` on CPU `cpu_id`
    /// (0 = ARM9). Returns the cycles taken.
    ///
    /// ```text
    /// from: arm_instruction::swi / thumb SWI, when config.hle_bios is set
    ///       (otherwise the CPU jumps into the BIOS image: handle_swi)
    ///   |
    ///   v
    /// hle_bios(cpu_id)
    ///   wrap the CPU + bus in `Caller` (SwiContext) and call
    ///   ds_bios::swi::arm9 / arm7, which decodes the SWI number and runs
    ///   Div, CpuSet, LZ77/Huffman decompression, Halt, IntrWait, CRC...
    ///   using reg()/set_reg() and read*/write* (normal bus routing)
    ///   |
    ///   v
    /// to:   back to the interpreter, which continues after the SWI
    /// ```
    pub fn hle_bios(&mut self, cpu_id: i32) -> i32 {
        if cpu_id == 0 {
            swi::arm9(&mut Caller { emu: self, cpu: CpuType::Arm9 })
        } else {
            swi::arm7(&mut Caller { emu: self, cpu: CpuType::Arm7 })
        }
    }
}
