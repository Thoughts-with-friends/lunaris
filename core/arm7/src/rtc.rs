/// Real Time Clock (RTC) for Nintendo DS
/// Manages date, time, and alarm functionality via bit-banged serial protocol

/// Alarm settings
#[derive(Debug, Clone, Copy)]
pub struct Alarm {
    /// Day of week (0=Sunday, 6=Saturday)
    pub day_of_week: u8,
    /// Hour (0-23)
    pub hour: u8,
    /// Minute (0-59)
    pub minute: u8,
}

impl Alarm {
    /// Create new alarm
    pub fn new() -> Self {
        Alarm {
            day_of_week: 0,
            hour: 0,
            minute: 0,
        }
    }
}

impl Default for Alarm {
    fn default() -> Self {
        Self::new()
    }
}

/// Real Time Clock
/// Provides date/time tracking and alarm functionality
pub struct RealTimeClock {
    /// Status register 1
    stat1_reg: u8,
    /// Status register 2
    stat2_reg: u8,

    /// Current year (0-99, representing 2000-2099)
    year: u8,
    /// Current month (1-12)
    month: u8,
    /// Current day (1-31)
    day: u8,
    /// Current day of week (0=Sunday, 6=Saturday)
    day_of_week: u8,

    /// Current hour (0-23)
    hour: u8,
    /// Current minute (0-59)
    minute: u8,
    /// Current second (0-59)
    second: u8,

    /// Alarm 1 settings
    alarm1: Alarm,
    /// Alarm 2 settings
    alarm2: Alarm,

    // Bit-banging I/O state
    /// I/O register for serial communication
    io_reg: u16,
    /// Internal output buffer for serial data
    internal_output: [u8; 7],
    /// Current command being processed
    command: u32,
    /// Input data accumulator
    input: u32,
    /// Current bit position in input
    input_bit_num: u32,
    /// Index in input buffer
    input_index: u32,
    /// Current bit position in output
    output_bit_num: u32,
    /// Index in output buffer
    output_index: u32,
}

impl RealTimeClock {
    /// Create new RTC
    pub fn new() -> Self {
        RealTimeClock {
            stat1_reg: 0,
            stat2_reg: 0,

            year: 0,
            month: 1,
            day: 1,
            day_of_week: 0,

            hour: 0,
            minute: 0,
            second: 0,

            alarm1: Alarm::new(),
            alarm2: Alarm::new(),

            io_reg: 0,
            internal_output: [0u8; 7],
            command: 0,
            input: 0,
            input_bit_num: 0,
            input_index: 0,
            output_bit_num: 0,
            output_index: 0,
        }
    }

    /// Initialize RTC with default values
    pub fn init(&mut self) -> Result<(), String> {
        // Initialize to default date/time (2000-01-01 00:00:00)
        self.year = 0;
        self.month = 1;
        self.day = 1;
        self.day_of_week = 6; // Saturday
        self.hour = 0;
        self.minute = 0;
        self.second = 0;

        self.stat1_reg = 0;
        self.stat2_reg = 0;

        Ok(())
    }

    /// Process input data from serial protocol
    pub fn interpret_input(&mut self) -> Result<(), String> {
        // Parse command and data from bit-banged input
        // Different commands: read, write, status, etc.
        Ok(())
    }

    /// Read from RTC
    pub fn read(&self) -> u16 {
        self.io_reg
    }

    /// Write to RTC
    pub fn write(&mut self, value: u16, is_byte: bool) -> Result<(), String> {
        if is_byte {
            // Byte write
            self.io_reg = (self.io_reg & 0xFF00) | (value & 0xFF);
        } else {
            // Word write
            self.io_reg = value;
        }
        Ok(())
    }

    // Getter methods

    /// Get current year (0-99)
    pub fn get_year(&self) -> u8 {
        self.year
    }

    /// Get current month (1-12)
    pub fn get_month(&self) -> u8 {
        self.month
    }

    /// Get current day (1-31)
    pub fn get_day(&self) -> u8 {
        self.day
    }

    /// Get current day of week (0=Sun, 6=Sat)
    pub fn get_day_of_week(&self) -> u8 {
        self.day_of_week
    }

    /// Get current hour (0-23)
    pub fn get_hour(&self) -> u8 {
        self.hour
    }

    /// Get current minute (0-59)
    pub fn get_minute(&self) -> u8 {
        self.minute
    }

    /// Get current second (0-59)
    pub fn get_second(&self) -> u8 {
        self.second
    }

    // Setter methods

    /// Set year (0-99 for 2000-2099)
    pub fn set_year(&mut self, year: u8) {
        self.year = year;
    }

    /// Set month (1-12)
    pub fn set_month(&mut self, month: u8) {
        self.month = month.min(12).max(1);
    }

    /// Set day (1-31)
    pub fn set_day(&mut self, day: u8) {
        self.day = day.min(31).max(1);
    }

    /// Set day of week (0=Sun, 6=Sat)
    pub fn set_day_of_week(&mut self, dow: u8) {
        self.day_of_week = dow % 7;
    }

    /// Set hour (0-23)
    pub fn set_hour(&mut self, hour: u8) {
        self.hour = hour % 24;
    }

    /// Set minute (0-59)
    pub fn set_minute(&mut self, minute: u8) {
        self.minute = minute % 60;
    }

    /// Set second (0-59)
    pub fn set_second(&mut self, second: u8) {
        self.second = second % 60;
    }

    // Alarm methods

    /// Get alarm 1
    pub fn get_alarm1(&self) -> Alarm {
        self.alarm1
    }

    /// Set alarm 1
    pub fn set_alarm1(&mut self, alarm: Alarm) {
        self.alarm1 = alarm;
    }

    /// Get alarm 2
    pub fn get_alarm2(&self) -> Alarm {
        self.alarm2
    }

    /// Set alarm 2
    pub fn set_alarm2(&mut self, alarm: Alarm) {
        self.alarm2 = alarm;
    }

    // Status register methods

    /// Get status register 1
    pub fn get_stat1(&self) -> u8 {
        self.stat1_reg
    }

    /// Set status register 1
    pub fn set_stat1(&mut self, value: u8) {
        self.stat1_reg = value;
    }

    /// Get status register 2
    pub fn get_stat2(&self) -> u8 {
        self.stat2_reg
    }

    /// Set status register 2
    pub fn set_stat2(&mut self, value: u8) {
        self.stat2_reg = value;
    }

    // Helper methods

    /// Increment second and handle overflow
    pub fn tick_second(&mut self) {
        self.second += 1;
        if self.second >= 60 {
            self.second = 0;
            self.tick_minute();
        }
    }

    /// Increment minute and handle overflow
    fn tick_minute(&mut self) {
        self.minute += 1;
        if self.minute >= 60 {
            self.minute = 0;
            self.tick_hour();
        }
    }

    /// Increment hour and handle overflow
    fn tick_hour(&mut self) {
        self.hour += 1;
        if self.hour >= 24 {
            self.hour = 0;
            self.tick_day();
        }
    }

    /// Increment day and handle overflow
    fn tick_day(&mut self) {
        self.day_of_week = (self.day_of_week + 1) % 7;

        let days_in_month = match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                // Simple leap year check (century rules ignored for simplicity)
                if (self.year as u32 + 2000) % 4 == 0 {
                    29
                } else {
                    28
                }
            }
            _ => 31,
        };

        self.day += 1;
        if self.day > days_in_month {
            self.day = 1;
            self.tick_month();
        }
    }

    /// Increment month and handle overflow
    fn tick_month(&mut self) {
        self.month += 1;
        if self.month > 12 {
            self.month = 1;
            self.tick_year();
        }
    }

    /// Increment year
    fn tick_year(&mut self) {
        self.year = (self.year + 1) % 100;
    }

    /// Convert time to BCD format (for I2C protocol)
    pub fn to_bcd(value: u8) -> u8 {
        ((value / 10) << 4) | (value % 10)
    }

    /// Convert BCD to binary
    pub fn from_bcd(value: u8) -> u8 {
        ((value >> 4) * 10) + (value & 0xF)
    }
}

impl Default for RealTimeClock {
    fn default() -> Self {
        Self::new()
    }
}
