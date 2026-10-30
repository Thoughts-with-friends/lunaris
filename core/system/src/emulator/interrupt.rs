// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! emulator.hpp
//!
use ds_interrupt::Interrupt;
use ds_peripherals::ipc::{IpcIrq, IrqRequest, Side};

use crate::emulator::Emulator;

impl Emulator {
    /// Requests ARM7 IRQ 24 when the Wi-Fi controller's `W_IF & W_IE` rose.
    ///
    /// ```text
    /// from: run() step 2d (after wifi.run) / arm7_write_halfword|word to
    ///       0x04800000.. (W_IE, W_IF_SET...)
    /// to:   IF7 bit 24 -> execute() takes the IRQ on the ARM7
    /// ```
    pub fn raise_wifi_irq(&mut self) {
        if self.wifi.take_irq() {
            self.request_interrupt7(Interrupt::Wifi);
        }
    }

    /// Raises an IRQ produced as a side effect of an IPC register access.
    ///
    /// ```text
    /// from: IPCSYNC / IPCFIFOCNT / IPCFIFOSEND writes and IPCFIFORECV
    ///       reads (read_/write_arm7|arm9), which return the request
    /// to:   request_interrupt7/9 (IPC sync 16, send empty 17, recv not
    ///       empty 18) on the CPU named in the request
    /// ```
    pub fn raise_ipc(&mut self, request: Option<IrqRequest>) {
        let Some((side, irq)) = request else {
            return;
        };
        let id = match irq {
            IpcIrq::Sync => Interrupt::IpcSync,
            IpcIrq::SendEmpty => Interrupt::IpcFifoEmpty,
            IpcIrq::RecvNotEmpty => Interrupt::IpcFifoNempty,
        };
        match side {
            Side::Arm9 => self.request_interrupt9(id),
            Side::Arm7 => self.request_interrupt7(id),
        }
    }

    /// Sets bit `id` of IF7. The ARM7 takes it in execute() once IE7 and
    /// IME7 allow; a halted ARM7 wakes as soon as IE7 & IF7 != 0.
    pub fn request_interrupt7(&mut self, id: Interrupt) {
        self.int7_reg.irq_flags |= 1 << (id as u32);
    }

    /// Sets bit `id` of IF9 (see [`Self::request_interrupt7`]).
    pub fn request_interrupt9(&mut self, id: Interrupt) {
        self.int9_reg.irq_flags |= 1 << (id as u32);
    }

    /// Sets IF7 bit `id` (GBA-slot interrupts are ARM7 interrupts).
    pub fn request_interrupt_gba(&mut self, id: i32) {
        self.int7_reg.irq_flags |= 1 << (id as u32);
    }

    /// Whether EXMEMCNT bit 11 gives the NDS slot to the ARM7 (selects the
    /// CPU whose cartridge DMA gamecart_request() starts).
    pub fn arm7_has_cart_rights(&self) -> bool {
        // DS Memory Control - Cartridges and Main RAM: https://problemkaputt.de/gbatek.htm#dsmemorycontrolcartridgesandmainram
        (self.ex_mem_cnt & (1 << 11)) != 0 // EXMEMCNT bit 11: NDS slot access rights (1 = ARM7)
    }
}
