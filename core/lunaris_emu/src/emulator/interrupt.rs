// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! emulator.hpp
//!
use crate::emulator::Emulator;
use crate::interrupts::Interrupt;

impl Emulator {
    /// Request an interrupt for ARM7.
    pub fn request_interrupt7(&mut self, id: Interrupt) {
        self.int7_reg.irq_flags |= 1 << (id as u32);
    }

    /// Request an interrupt for ARM9.
    pub fn request_interrupt9(&mut self, id: Interrupt) {
        self.int9_reg.irq_flags |= 1 << (id as u32);
    }

    /// Request a GBA interrupt.
    pub fn request_interrupt_gba(&mut self, id: i32) {
        self.int7_reg.irq_flags |= 1 << (id as u32);
    }

    /// Check if ARM7 has cartridge access rights.
    pub fn arm7_has_cart_rights(&self) -> bool {
        (self.ex_mem_cnt & (1 << 11)) != 0
    }
}
