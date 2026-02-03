use crate::emulator::Emulator;
use crate::interrupts::Interrupt;

impl Emulator {
    /// Handle timer overflow
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
                    #[cfg(feature = "tracing")]
                    tracing::warn!("Invalid Interrupt value: {id}");
                }
            } else {
                let id = (Interrupt::Timer0 as usize) + index - 4;
                if let Some(id) = Interrupt::from_usize(id) {
                    self.request_interrupt9(id as Interrupt);
                } else {
                    #[cfg(feature = "tracing")]
                    tracing::warn!("Invalid Interrupt value: {id}");
                }
            }
        }

        //Count-up timing behavior
        if index != 3 && index != 7 {
            let count_up_timing = self.nds_timing.timers[index + 1].count_up_timing;
            let enabled = self.nds_timing.timers[index + 1].enabled;
            let count_up = count_up_timing && enabled;

            if count_up {
                if self.nds_timing.timers[index + 1].counter == 0xFFFF {
                    self.overflow(index + 1); //recursion baby
                } else {
                    self.nds_timing.timers[index + 1].counter += 1;
                }
            }
        }
    }

    /// Run ARM9 timers for specified cycles
    pub fn run_timers9(&mut self, cycles: i32) {
        // Run timers 0-3 (ARM9)
        for i in 0..4 {
            if self.nds_timing.timers[i].enabled {
                self.run_timer(cycles, i);
            }
        }
    }

    /// Run ARM7 timers for specified cycles
    pub fn run_timers7(&mut self, cycles: i32) {
        // Run timers 4-7 (ARM7)
        for i in 4..8 {
            if self.nds_timing.timers[i].enabled {
                self.run_timer(cycles, i);
            }
        }
    }

    /// Run individual timer
    pub fn run_timer(&mut self, cycles: i32, index: usize) {
        if !self.nds_timing.timers[index].count_up_timing {
            self.nds_timing.timers[index].cycles_left -= cycles;
            let old_timer = self.nds_timing.timers[index].counter;

            while self.nds_timing.timers[index].cycles_left <= 0 {
                self.nds_timing.timers[index].counter += 1;
                self.nds_timing.timers[index].cycles_left += self.nds_timing.timer_clock_divs
                    [self.nds_timing.timers[index].clock_div as usize];
            }

            if self.nds_timing.timers[index].counter < old_timer {
                self.overflow(index);
            }
        }
    }
}
