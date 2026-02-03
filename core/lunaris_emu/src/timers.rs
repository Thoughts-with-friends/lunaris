//! Timer system for Nintendo DS
//! Manages 8 timers (4 per CPU) with frequency division and overflow interrupts

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
#[derive(Debug)]
pub struct NDSTiming {
    /// Timer clock divisor values for each of 4 timers
    pub(crate) timer_clock_divs: [i32; 4],

    /// Timer registers (0-3 for ARM9, 4-7 for ARM7)
    pub(crate) timers: [TimerReg; 8],
}

impl Default for NDSTiming {
    fn default() -> Self {
        Self::new()
    }
}

impl NDSTiming {
    /// Create new timing system
    pub fn new() -> Self {
        NDSTiming {
            timer_clock_divs: [0; 4],
            timers: [TimerReg::new(); 8],
        }
    }

    /// Power on timing system
    pub fn power_on(&mut self) {
        for timer in &mut self.timers {
            *timer = TimerReg::new();
        }
    }

    /// Read timer counter low byte (0x4000100 + index*4)
    pub fn read_lo(&self, index: usize) -> u16 {
        if index < 8 {
            self.timers[index].counter
        } else {
            #[cfg(feature = "tracing")]
            tracing::warn!("fn read_lo: out-of bounds {index}");
            0 // to prevent out-of bounds
        }
    }

    /// Read timer control high byte
    pub fn read_hi(&self, index: usize) -> u16 {
        if index < 8 {
            self.timers[index].get_control()
        } else {
            #[cfg(feature = "tracing")]
            tracing::warn!("fn read_hi: out-of bounds {index}");
            0
        }
    }

    /// Write 32-bit word to timer
    pub fn write(&mut self, value: u32, index: usize) {
        self.write_lo((value & 0xFFFF) as u16, index);
        self.write_hi((value >> 16) as u16, index);
    }

    /// Write counter value (low 16 bits)
    pub fn write_lo(&mut self, value: u16, index: usize) {
        self.timers[index].reload_value = value;
    }

    /// Write control register (high 16 bits)
    pub fn write_hi(&mut self, value: u16, index: usize) {
        let timer = &mut self.timers[index];

        match value & 0x3 {
            0 => {
                timer.clock_div = Divisor::F1;
                timer.cycles_left = 1;
            }
            1 => {
                timer.clock_div = Divisor::F64;
                timer.cycles_left = 64;
            }
            2 => {
                timer.clock_div = Divisor::F256;
                timer.cycles_left = 256;
            }
            3 => {
                timer.clock_div = Divisor::F1024;
                timer.cycles_left = 1024;
            }
            _ => unreachable!(),
        }

        timer.count_up_timing = (value & (1 << 2)) != 0;
        timer.irq_on_overflow = (value & (1 << 6)) != 0;

        let enable = (value & (1 << 7)) != 0;

        // If timer is being newly enabled, reload the counter
        if !timer.enabled && enable {
            timer.counter = timer.reload_value;
        }

        timer.enabled = enable;
    }
}
