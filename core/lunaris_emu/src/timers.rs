/// Timer system for Nintendo DS
/// Manages 8 timers (4 per CPU) with frequency division and overflow interrupts

/// Timer frequency divisor enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divisor {
    /// F = system clock (33.513982 MHz on NDS)
    F1 = 0,
    /// Divide by 64
    F64 = 1,
    /// Divide by 256
    F256 = 2,
    /// Divide by 1024
    F1024 = 3,
}

impl Divisor {
    /// Get the divisor value as integer
    pub fn value(&self) -> u32 {
        match self {
            Divisor::F1 => 1,
            Divisor::F64 => 64,
            Divisor::F256 => 256,
            Divisor::F1024 => 1024,
        }
    }

    /// Convert numeric value to Divisor
    pub fn from_value(val: u32) -> Self {
        match val & 0x3 {
            0 => Divisor::F1,
            1 => Divisor::F64,
            2 => Divisor::F256,
            3 => Divisor::F1024,
            _ => Divisor::F1,
        }
    }
}

/// Individual timer register
#[derive(Debug, Clone, Copy)]
pub struct TimerReg {
    /// Current counter value
    pub counter: u16,
    /// Value to reload counter on overflow
    pub reload_value: u16,
    /// Cycles remaining before increment
    pub cycles_left: i32,
    /// Clock frequency divisor
    pub clock_div: Divisor,
    /// Count up mode: increment when previous timer overflows
    pub count_up_timing: bool,
    /// Generate interrupt on overflow
    pub irq_on_overflow: bool,
    /// Timer enabled
    pub enabled: bool,
}

impl TimerReg {
    /// Create new timer register
    pub fn new() -> Self {
        TimerReg {
            counter: 0,
            reload_value: 0,
            cycles_left: 0,
            clock_div: Divisor::F1,
            count_up_timing: false,
            irq_on_overflow: false,
            enabled: false,
        }
    }

    /// Get control register value
    pub fn get_control(&self) -> u16 {
        let mut value = 0u16;
        value |= (self.clock_div as u16) & 0x3;
        if self.count_up_timing {
            value |= 1 << 2;
        }
        if self.irq_on_overflow {
            value |= 1 << 6;
        }
        if self.enabled {
            value |= 1 << 7;
        }
        value
    }

    /// Set control register value
    pub fn set_control(&mut self, value: u16) {
        self.clock_div = Divisor::from_value((value & 0x3) as u32);
        self.count_up_timing = (value & (1 << 2)) != 0;
        self.irq_on_overflow = (value & (1 << 6)) != 0;
        self.enabled = (value & (1 << 7)) != 0;
    }
}

impl Default for TimerReg {
    fn default() -> Self {
        Self::new()
    }
}

/// NDS Timing and Timer system
/// Manages 8 timers total (4 for ARM9, 4 for ARM7)
pub struct NDSTiming {
    /// Emulator reference
    emulator_ptr: *mut crate::emulator::Emulator,

    /// Timer clock divisor values for each of 4 timers
    timer_clock_divs: [i32; 4],

    /// Timer registers (0-3 for ARM9, 4-7 for ARM7)
    timers: [TimerReg; 8],
}

impl NDSTiming {
    /// Create new timing system
    pub fn new() -> Self {
        NDSTiming {
            emulator_ptr: std::ptr::null_mut(),
            timer_clock_divs: [0; 4],
            timers: [TimerReg::new(); 8],
        }
    }

    /// Power on timing system
    pub fn power_on(&mut self) -> Result<(), String> {
        for timer in &mut self.timers {
            *timer = TimerReg::new();
        }
        Ok(())
    }

    /// Run ARM9 timers for specified cycles
    pub fn run_timers9(&mut self, cycles: i32) -> Result<(), String> {
        // Run timers 0-3 (ARM9)
        for i in 0..4 {
            self.run_timer(cycles, i)?;
        }
        Ok(())
    }

    /// Run ARM7 timers for specified cycles
    pub fn run_timers7(&mut self, cycles: i32) -> Result<(), String> {
        // Run timers 4-7 (ARM7)
        for i in 4..8 {
            self.run_timer(cycles, i)?;
        }
        Ok(())
    }

    /// Run individual timer
    pub fn run_timer(&mut self, cycles: i32, index: usize) -> Result<(), String> {
        if index >= 8 {
            return Err("Invalid timer index".to_string());
        }

        if !self.timers[index].enabled {
            return Ok(());
        }

        // Count-up timing mode: increment when previous timer overflows
        if self.timers[index].count_up_timing && index > 0 {
            // This is handled by the overflow() function of the previous timer
            return Ok(());
        }

        // Calculate how many timer increments occur in this cycle count
        let divisor = self.timers[index].clock_div.value() as i32;
        let mut remaining_cycles = cycles;

        while remaining_cycles > 0 {
            if self.timer_clock_divs[index % 4] >= divisor {
                // Timer increments
                self.timers[index].counter = self.timers[index].counter.wrapping_add(1);
                self.timer_clock_divs[index % 4] = 0;

                // Check for overflow
                if self.timers[index].counter == 0 {
                    self.overflow(index)?;
                }
            }

            self.timer_clock_divs[index % 4] += 1;
            remaining_cycles -= 1;
        }

        Ok(())
    }

    /// Handle timer overflow
    fn overflow(&mut self, index: usize) -> Result<(), String> {
        if index >= 8 {
            return Ok(());
        }

        // Reload counter with reload value
        self.timers[index].counter = self.timers[index].reload_value;

        // If interrupt on overflow is enabled, request interrupt
        if self.timers[index].irq_on_overflow {
            // Request interrupt (implementation depends on interrupt controller)
        }

        // If next timer has count-up timing, increment it
        if index < 7 && self.timers[index + 1].count_up_timing && self.timers[index + 1].enabled {
            self.timers[index + 1].counter = self.timers[index + 1].counter.wrapping_add(1);
            if self.timers[index + 1].counter == 0 {
                self.overflow(index + 1)?;
            }
        }

        Ok(())
    }

    /// Read timer counter low byte (0x4000100 + index*4)
    pub fn read_lo(&self, index: usize) -> u16 {
        if index < 8 {
            self.timers[index].counter
        } else {
            0
        }
    }

    /// Read timer control high byte
    pub fn read_hi(&self, index: usize) -> u16 {
        if index < 8 {
            self.timers[index].get_control()
        } else {
            0
        }
    }

    /// Write 32-bit word to timer
    pub fn write(&mut self, value: u32, index: usize) {
        if index < 8 {
            self.timers[index].counter = (value & 0xFFFF) as u16;
            self.timers[index].set_control(((value >> 16) & 0xFFFF) as u16);
            self.timer_clock_divs[index % 4] = 0;
        }
    }

    /// Write counter value (low 16 bits)
    pub fn write_lo(&mut self, value: u16, index: usize) {
        if index < 8 {
            self.timers[index].reload_value = value;
            // When writing to counter, also reset the divisor
            self.timer_clock_divs[index % 4] = 0;
        }
    }

    /// Write control register (high 16 bits)
    pub fn write_hi(&mut self, value: u16, index: usize) {
        if index < 8 {
            let was_enabled = self.timers[index].enabled;
            self.timers[index].set_control(value);

            // If timer was just enabled, reload and reset divisor
            if !was_enabled && self.timers[index].enabled {
                self.timers[index].counter = self.timers[index].reload_value;
                self.timer_clock_divs[index % 4] = 0;
            }
        }
    }

    /// Get timer counter value
    pub fn get_counter(&self, index: usize) -> u16 {
        if index < 8 {
            self.timers[index].counter
        } else {
            0
        }
    }

    /// Set timer counter value
    pub fn set_counter(&mut self, index: usize, value: u16) {
        if index < 8 {
            self.timers[index].counter = value;
            self.timer_clock_divs[index % 4] = 0;
        }
    }

    /// Get timer reload value
    pub fn get_reload(&self, index: usize) -> u16 {
        if index < 8 {
            self.timers[index].reload_value
        } else {
            0
        }
    }

    /// Set timer reload value
    pub fn set_reload(&mut self, index: usize, value: u16) {
        if index < 8 {
            self.timers[index].reload_value = value;
        }
    }

    /// Check if timer is enabled
    pub fn is_enabled(&self, index: usize) -> bool {
        if index < 8 {
            self.timers[index].enabled
        } else {
            false
        }
    }

    /// Get timer reference
    pub fn get_timer(&self, index: usize) -> Option<&TimerReg> {
        if index < 8 {
            Some(&self.timers[index])
        } else {
            None
        }
    }

    /// Get mutable timer reference
    pub fn get_timer_mut(&mut self, index: usize) -> Option<&mut TimerReg> {
        if index < 8 {
            Some(&mut self.timers[index])
        } else {
            None
        }
    }
}

impl Default for NDSTiming {
    fn default() -> Self {
        Self::new()
    }
}
