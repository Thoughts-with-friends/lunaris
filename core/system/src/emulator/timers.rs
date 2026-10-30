use ds_interrupt::Interrupt;

use crate::emulator::Emulator;

impl Emulator {
    /// Timer `index` (0-3 ARM7, 4-7 ARM9) passed 0xFFFF.
    ///
    /// ```text
    /// from: run_timer() / overflow() of the previous timer (count-up)
    ///   |
    ///   v
    /// overflow(index)
    ///   1. counter = reload value
    ///   2. IRQ enabled -> request_interrupt7/9(Timer0 + n)
    ///   3. next timer in count-up mode -> +1 (and overflow() again if
    ///      it wraps)
    /// ```
    pub fn overflow(&mut self, index: usize) {
        // Reload counter with reload value
        self.nds_timing.timers[index].counter = self.nds_timing.timers[index].reload_value;

        // Overflow -> request interrupt
        if self.nds_timing.timers[index].irq_on_overflow {
            if index < 4 {
                let id = (Interrupt::Timer0 as usize) + index;
                if let Some(id) = Interrupt::from_usize(id) {
                    self.request_interrupt7(id);
                } else {
                    #[cfg(feature = "log")]
                    log::warn!("Invalid Interrupt value: {id}");
                }
            } else {
                let id = (Interrupt::Timer0 as usize) + index - 4;
                if let Some(id) = Interrupt::from_usize(id) {
                    self.request_interrupt9(id as Interrupt);
                } else {
                    #[cfg(feature = "log")]
                    log::warn!("Invalid Interrupt value: {id}");
                }
            }
        }

        // Count-up timing behavior
        if index != 3 && index != 7 {
            let count_up_timing = self.nds_timing.timers[index + 1].count_up_timing;
            let enabled = self.nds_timing.timers[index + 1].enabled;
            let count_up = count_up_timing && enabled;

            if count_up {
                // DS Timers: https://problemkaputt.de/gbatek.htm#dstimers
                if self.nds_timing.timers[index + 1].counter == 0xFFFF {
                    // count-up timer about to overflow
                    self.overflow(index + 1);
                } else {
                    self.nds_timing.timers[index + 1].counter += 1;
                }
            }
        }
    }

    /// Run ARM9 timers for specified cycles (bus clock); called from run()
    /// step 2b after every ARM9 instruction.
    ///
    /// Timer slots 0-3 belong to the ARM7 and 4-7 to the ARM9, matching the
    /// register handlers and the IRQ routing in [`Self::overflow`].
    pub fn run_timers9(&mut self, cycles: i32) {
        for i in 4..8 {
            if self.nds_timing.timers[i].enabled {
                self.run_timer(cycles, i);
            }
        }
    }

    /// Run ARM7 timers for specified cycles; called from run() step 2c after
    /// every ARM7 instruction.
    pub fn run_timers7(&mut self, cycles: i32) {
        for i in 0..4 {
            if self.nds_timing.timers[i].enabled {
                self.run_timer(cycles, i);
            }
        }
    }

    /// Run individual timer. Every elapsed prescaler period increments the
    /// counter; incrementing past 0xFFFF reloads it and raises the overflow
    /// (which may happen several times within one call).
    ///
    /// ```text
    /// from: run_timers7 / run_timers9 (enabled timers only)
    ///   1. count-up timers (not 0/4) return: overflow() drives them
    ///   2. cycles_left -= cycles; per elapsed prescaler period (1, 64,
    ///      256 or 1024 cycles): counter += 1, or overflow() at 0xFFFF
    /// to:   overflow() -> timer IRQ / cascaded timer
    /// ```
    pub fn run_timer(&mut self, cycles: i32, index: usize) {
        if self.nds_timing.timers[index].count_up_timing && !index.is_multiple_of(4) {
            return; // driven by the previous timer's overflow
        }

        let div =
            self.nds_timing.timer_clock_divs[self.nds_timing.timers[index].clock_div as usize];
        self.nds_timing.timers[index].cycles_left -= cycles;
        while self.nds_timing.timers[index].cycles_left <= 0 {
            self.nds_timing.timers[index].cycles_left += div;
            let timer = &mut self.nds_timing.timers[index];
            if timer.counter == 0xFFFF {
                // counter about to overflow
                self.overflow(index);
            } else {
                timer.counter += 1;
            }
        }
    }
}
