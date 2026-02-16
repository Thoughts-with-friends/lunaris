//! WiFi Controller stub for Nintendo DS
//! Provides basic WiFi hardware register interface (emulation stub)

/// WiFi Controller
/// This is a stub implementation for WiFi hardware
/// Full WiFi emulation is not currently implemented
#[derive(Debug)]
pub struct WiFi {
    /// Power control register (0x0236)
    w_power_us: u16,

    /// Baseband write register (0x015A)
    w_bb_write: u16,
    /// Baseband read register (0x015C)
    w_bb_read: u16,
    /// Baseband mode register (0x0160)
    w_bb_mode: u16,
    /// Baseband power register (0x0168)
    w_bb_power: u16,

    /// RF control register (0x0184)
    w_rf_cnt: u16,

    /// Baseband busy flag
    bb_busy: bool,
    /// RF busy flag
    rf_busy: bool,
}

impl Default for WiFi {
    fn default() -> Self {
        Self::new()
    }
}

impl WiFi {
    /// Create new WiFi controller
    pub fn new() -> Self {
        WiFi {
            w_power_us: 0,
            w_bb_write: 0,
            w_bb_read: 0,
            w_bb_mode: 0,
            w_bb_power: 0,
            w_rf_cnt: 0,
            bb_busy: false,
            rf_busy: false,
        }
    }

    /// Perform baseband read operation
    fn bb_read(&mut self, index: u16) {
        match index {
            0 => self.w_bb_read = 0x6D,
            _ => self.w_bb_read = 0,
        }
    }

    /// Perform baseband write operation
    fn bb_write(&mut self, index: u16) {
        let _ = index;
        todo!()
    }

    /// Set power control register
    pub fn set_w_power_us(&mut self, value: u16) {
        self.w_power_us = value;
    }

    /// Set baseband control register
    pub fn set_w_bb_cnt(&mut self, value: u16) {
        // Baseband count/control
        let index = value & 0xFF;
        let direction = value >> 12;

        if direction == 5 {
            self.bb_write(index);
        } else if direction == 6 {
            self.bb_read(index);
        }
    }

    /// Set baseband write register
    pub fn set_w_bb_write(&mut self, value: u16) {
        self.w_bb_write = value;
    }

    /// Set baseband mode register
    pub fn set_w_bb_mode(&mut self, value: u16) {
        self.w_bb_mode = value;
    }

    /// Set baseband power register
    pub fn set_w_bb_power(&mut self, value: u16) {
        self.w_bb_power = value;
    }

    /// Set RF control register
    pub fn set_w_rf_cnt(&mut self, value: u16) {
        self.w_rf_cnt = value;
    }

    /// Check if RF is busy
    pub fn get_w_rf_busy(&self) -> bool {
        self.rf_busy
    }

    /// Get baseband read register
    pub fn get_w_bb_read(&self) -> u16 {
        self.w_bb_read
    }

    /// Check if baseband is busy
    pub fn get_w_bb_busy(&self) -> bool {
        self.bb_busy
    }
}
