//! NDS interrupt controller (IE / IF / IME registers).
//!
//! Each CPU has its own independent set of registers:
//! - **ARM9**: IME=4000208h, IE=4000210h, IF=4000214h
//! - **ARM7**: IME=4000208h, IE=4000200h, IF=4000202h
//!
//! Bits 0-13 mirror the GBA layout; bits 16-21 are NDS-specific.
//!
//! GBATEK "DS Interrupts" (IME/IE/IF, full bit table):
//! <https://problemkaputt.de/gbatek.htm#dsinterrupts>

use super::{Scheduler, mem::IORegister};
use bitflags::*;

#[derive(emu_utils::Savestate)]
pub struct InterruptController {
    pub enable: InterruptEnable,
    pub master_enable: InterruptMasterEnable,
    pub request: InterruptRequest,
}

impl InterruptController {
    pub fn new() -> Self {
        InterruptController {
            enable: InterruptEnable::empty(),
            master_enable: InterruptMasterEnable::empty(),
            request: InterruptRequest::empty(),
        }
    }

    /// Returns `true` when at least one enabled interrupt is pending.
    ///
    /// `ignore_ime`: ARM7 checks IME normally; ARM9 HALT state ignores IME
    /// because the CP15 halt is cleared by any enabled IRQ regardless of the
    /// global enable bit.  GBATEK "DS Interrupts – halt notes":
    /// <https://problemkaputt.de/gbatek.htm#dsinterrupts>
    pub fn interrupts_requested(&self, ignore_ime: bool) -> bool {
        (ignore_ime || self.master_enable.bits() != 0)
            && (self.request.bits() & self.enable.bits()) != 0
    }
}

bitflags! {
    /// IE (Interrupt Enable) register – 32-bit, same bit layout as IF.
    ///
    /// Bits 0-13: shared with GBA layout.
    /// Bits 14-15: unused on NDS (reserved).
    /// Bits 16-21: NDS-specific additions.
    ///
    /// GBATEK "DS Interrupts – interrupt sources table":
    /// <https://problemkaputt.de/gbatek.htm#dsinterrupts>
    pub struct InterruptEnable: u32 {
        const VBLANK = 1 << 0;           // LCD V-Blank
        const HBLANK = 1 << 1;           // LCD H-Blank
        const VCOUNTER_MATCH = 1 << 2;   // LCD V-Counter match (DISPSTAT LYC)
        const TIMER0_OVERFLOW = 1 << 3;
        const TIMER1_OVERFLOW = 1 << 4;
        const TIMER2_OVERFLOW = 1 << 5;
        const TIMER3_OVERFLOW = 1 << 6;
        const SERIAL = 1 << 7;           // SIO / UART (ARM7 only in practice)
        const DMA0 = 1 << 8;
        const DMA1 = 1 << 9;
        const DMA2 = 1 << 10;
        const DMA3 = 1 << 11;
        const KEYPAD = 1 << 12;          // Keypad – see KEYCNT for logic-AND/OR mode
        const GAME_PAK = 1 << 13;        // GBA cartridge slot (always 0 on retail NDS)
        // Bits 14-15: not used
        const IPC_SYNC = 1 << 16;              // IPCSYNC bit 14 sent by other CPU
        const IPC_SEND_FIFO_EMPTY = 1 << 17;   // Own send-FIFO transitioned to empty
        const IPC_RECV_FIFO_NOT_EMPTY = 1 << 18; // Other CPU's FIFO has data
        const GAME_CARD_TRANSFER_COMPLETION = 1 << 19; // NDS slot ROM block done
        const GAME_CARD_IREQ_MC = 1 << 20;     // NDS slot IREQ_MC line asserted
        const GEOMETRY_COMMAND_FIFO = 1 << 21; // 3-D GXFIFO below half-empty (ARM9 only)
    }
}

crate::impl_savestate_bitflags!(InterruptEnable);

bitflags! {
    pub struct InterruptMasterEnable: u32 {
        const ENABLE = 1 << 0;
    }
}

crate::impl_savestate_bitflags!(InterruptMasterEnable);

bitflags! {
    /// IF (Interrupt Request / Acknowledge) register – same bit layout as IE.
    ///
    /// Writing a `1` to a bit **clears** it (acknowledge).  This is opposite to
    /// typical registers; hardware fires an IRQ when `(IE & IF) != 0`.
    ///
    /// GBATEK "DS Interrupts – IF acknowledge behaviour":
    /// <https://problemkaputt.de/gbatek.htm#dsinterrupts>
    pub struct InterruptRequest: u32 {
        const VBLANK = 1 << 0;
        const HBLANK = 1 << 1;
        const VCOUNTER_MATCH = 1 << 2;
        const TIMER0_OVERFLOW = 1 << 3;
        const TIMER1_OVERFLOW = 1 << 4;
        const TIMER2_OVERFLOW = 1 << 5;
        const TIMER3_OVERFLOW = 1 << 6;
        const SERIAL = 1 << 7;
        const DMA0 = 1 << 8;
        const DMA1 = 1 << 9;
        const DMA2 = 1 << 10;
        const DMA3 = 1 << 11;
        const KEYPAD = 1 << 12;
        const GAME_PAK = 1 << 13;
        const IPC_SYNC = 1 << 16;
        const IPC_SEND_FIFO_EMPTY = 1 << 17;
        const IPC_RECV_FIFO_NOT_EMPTY = 1 << 18;
        const GAME_CARD_TRANSFER_COMPLETION = 1 << 19;
        const GAME_CARD_IREQ_MC = 1 << 20;
        // TODO: bit 21 (GEOMETRY_COMMAND_FIFO) is ARM9-only; ARM7 IF must never set it.
        const GEOMETRY_COMMAND_FIFO = 1 << 21;
    }
}

crate::impl_savestate_bitflags!(InterruptRequest);

impl IORegister for InterruptEnable {
    fn read(&self, byte: usize) -> u8 {
        match byte {
            0 => self.bits as u8,
            1 => (self.bits >> 8) as u8,
            2 => (self.bits >> 16) as u8,
            3 => (self.bits >> 24) as u8,
            _ => unreachable!(),
        }
    }

    fn write(&mut self, _scheduler: &mut Scheduler, byte: usize, value: u8) {
        match byte {
            0 => self.bits = self.bits & !0x0000_00FF | (value as u32),
            1 => self.bits = self.bits & !0x0000_FF00 | (value as u32) << 8,
            2 => self.bits = self.bits & !0x00FF_0000 | (value as u32) << 16,
            3 => self.bits = self.bits & !0xFF00_0000 | (value as u32) << 24,
            _ => unreachable!(),
        }
    }
}

impl IORegister for InterruptMasterEnable {
    fn read(&self, byte: usize) -> u8 {
        match byte {
            0 => self.bits as u8,
            1 => (self.bits >> 8) as u8,
            2 => (self.bits >> 16) as u8,
            3 => (self.bits >> 24) as u8,
            _ => unreachable!(),
        }
    }

    fn write(&mut self, _scheduler: &mut Scheduler, byte: usize, value: u8) {
        match byte {
            0 => {
                self.bits = self.bits & !0x0000_00FF | (value as u32) & InterruptEnable::all().bits
            }
            1 => {
                self.bits =
                    self.bits & !0x0000_FF00 | (value as u32) << 8 & InterruptEnable::all().bits
            }
            2 => {
                self.bits =
                    self.bits & !0x00FF_0000 | (value as u32) << 16 & InterruptEnable::all().bits
            }
            3 => {
                self.bits =
                    self.bits & !0xFF00_0000 | (value as u32) << 24 & InterruptEnable::all().bits
            }
            _ => unreachable!(),
        }
    }
}

impl IORegister for InterruptRequest {
    /// Reads one byte of the IF register.
    fn read(&self, byte: usize) -> u8 {
        match byte {
            0 => self.bits as u8,
            1 => (self.bits >> 8) as u8,
            2 => (self.bits >> 16) as u8,
            3 => (self.bits >> 24) as u8,
            _ => unreachable!(),
        }
    }

    /// Acknowledges (clears) interrupt bits.
    ///
    /// Writing `1` to a bit clears that pending interrupt.
    ///
    /// GBATEK: <https://problemkaputt.de/gbatek.htm#dsinterrupts>
    fn write(&mut self, _scheduler: &mut Scheduler, byte: usize, value: u8) {
        match byte {
            0 => self.bits &= !(value as u32),
            1 => self.bits &= !((value as u32) << 8),
            2 => self.bits &= !((value as u32) << 16),
            3 => self.bits &= !((value as u32) << 24),
            _ => unreachable!(),
        }
    }
}
