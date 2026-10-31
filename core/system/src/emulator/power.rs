//! Keypad interrupt, lid (hinge), sleep mode and the RTC interrupt.
//!
//! ```text
//!  frontend:  button_*()  set_lid_closed()     run() start: power_frame_start()
//!                 |              |                  |
//!                 v              v                  v
//!  KEYCNT (0x04000132)   EXTKEYIN bit 7      check_key_irq(): IRQ 12 when the
//!  per CPU: bits 0-9     opening -> ARM7     KEYCNT condition becomes true
//!  keys, 14 IRQ on,      IRQ 22 (unfold)     rtc.tick(): RTC IRQ 7 (ARM7) when
//!  15 AND (else OR)                          RCNT routes it (bits 8, 14-15)
//!
//!  HALTCNT = C0h (ARM7): sleep. The ARM7 halts until an enabled IRQ (keypad,
//!  lid open, RTC); the screens show black meanwhile.
//! ```
//!
//! GBATEK: [DS Keypad](https://problemkaputt.de/gbatek.htm#dskeypad),
//! [DS Power Control](https://problemkaputt.de/gbatek.htm#dspowercontrol)
use ds_interrupt::Interrupt;

use crate::emulator::Emulator;

impl Emulator {
    /// KEYCNT write for the ARM9 (`cpu` 0) or ARM7 (1).
    pub fn write_keycnt(&mut self, cpu: usize, value: u16) {
        self.key_cnt[cpu] = value & 0xC3FF;
    }

    /// Whether `cpu`'s KEYCNT condition holds for the keys `keys`
    /// (KEYINPUT, 0 = pressed).
    fn key_condition(&self, cpu: usize, keys: u16) -> bool {
        let cnt = self.key_cnt[cpu];
        let mask = cnt & 0x03FF;
        if cnt & (1 << 15) != 0 {
            keys & mask == 0 // AND: all selected keys pressed
        } else {
            keys & mask != mask // OR: any selected key pressed
        }
    }

    /// Raises the keypad IRQ on a CPU whose KEYCNT condition just became
    /// true (melonDS `NDS::CheckKeyIRQ`).
    fn check_key_irq(&mut self) {
        let keys = self.key_input.get_value();
        let old = self.last_keys;
        self.last_keys = keys;
        for cpu in 0..2 {
            if self.key_cnt[cpu] & (1 << 14) == 0 {
                continue;
            }
            if !self.key_condition(cpu, old) && self.key_condition(cpu, keys) {
                if cpu == 0 {
                    self.request_interrupt9(Interrupt::Keypad);
                } else {
                    self.request_interrupt7(Interrupt::Keypad);
                }
            }
        }
    }

    /// Per-frame checks at the start of `run()`: keypad IRQ, RTC IRQ.
    pub(crate) fn power_frame_start(&mut self) {
        self.check_key_irq();
        // RCNT bits 14-15 = 2 (GPIO mode) with bit 8 (SI interrupt) set.
        if self.rtc.tick() && self.r_cnt & 0xC100 == 0x8100 {
            self.request_interrupt7(Interrupt::Rtc);
        }
    }

    /// Closes or opens the lid. Opening raises the ARM7 "screens
    /// unfolding" IRQ, which wakes a sleeping console.
    pub fn set_lid_closed(&mut self, closed: bool) {
        let was = self.ext_key_in.hinge_closed;
        self.ext_key_in.hinge_closed = closed;
        if was && !closed {
            self.request_interrupt7(Interrupt::UnfoldScreen);
        }
    }

    /// Whether the lid is closed.
    pub const fn lid_closed(&self) -> bool {
        self.ext_key_in.hinge_closed
    }

    /// HALTCNT = C0h: the ARM7 enters sleep mode.
    pub(crate) fn enter_sleep(&mut self) {
        self.sleeping = true;
        self.arm7.halt();
    }

    /// Whether the console is in sleep mode (screens dark).
    pub const fn is_sleeping(&self) -> bool {
        self.sleeping
    }

    /// Leaves sleep mode once the ARM7 has woken up.
    pub(crate) fn update_sleep(&mut self) {
        if self.sleeping && !self.arm7.halted {
            self.sleeping = false;
        }
    }

    /// Whether the software asked the power management chip to switch
    /// the console off (register 0 bit 6); the frontend may stop.
    pub fn power_off_requested(&self) -> bool {
        self.spi.power_off_requested()
    }

    /// Backlights (top, bottom) as set in the power management chip.
    pub fn backlights(&self) -> (bool, bool) {
        self.spi.backlights()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypad_irq_on_condition_edge() {
        let mut emu = Emulator::new();
        emu.power_on();
        emu.write_keycnt(1, (1 << 14) | 0x0001); // ARM7: IRQ on A (OR)
        emu.power_frame_start();
        assert_eq!(emu.int7_reg.irq_flags & (1 << 12), 0);
        emu.button_a_pressed();
        emu.power_frame_start();
        assert_ne!(emu.int7_reg.irq_flags & (1 << 12), 0);
    }

    #[test]
    fn opening_the_lid_wakes_the_arm7() {
        let mut emu = Emulator::new();
        emu.power_on();
        emu.set_lid_closed(true);
        emu.enter_sleep();
        assert!(emu.is_sleeping());
        emu.set_lid_closed(false);
        assert_ne!(emu.int7_reg.irq_flags & (1 << 22), 0);
    }
}
