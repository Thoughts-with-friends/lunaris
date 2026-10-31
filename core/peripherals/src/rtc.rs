// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! rtc.hpp
//!
/// Real Time Clock (RTC) for Nintendo DS
/// Manages date, time, and alarm functionality via bit-banged serial protocol
/// Alarm settings
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
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
    pub const fn new() -> Self {
        Self { day_of_week: 0, hour: 0, minute: 0 }
    }
}

impl Default for Alarm {
    fn default() -> Self {
        Self::new()
    }
}

// https://stackoverflow.com/questions/1408361/unsigned-integer-to-bcd-conversion
/// Binary 0-99 -> BCD.
const fn byte_to_bcd(byte: u8) -> u8 {
    (byte / 10 * 16) + (byte % 10)
}

/// Real Time Clock
/// Provides date/time tracking and alarm functionality
#[derive(serde::Serialize, serde::Deserialize, Debug)]
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

    /// When set, the clock always reads this time instead of the host's
    /// local time (deterministic runs for testing).
    pub fixed_time: Option<chrono::NaiveDateTime>,

    /// Seconds added to the host's local time (a clock set to another
    /// date keeps running from there).
    #[serde(default)]
    pub time_offset: i64,
    /// Clock adjustment register (command 3).
    clock_adjust: u8,
    /// Free register (command 7).
    free_reg: u8,
    /// Interrupt lines: bit 4 INT1, bit 5 INT2.
    irq_flag: u8,
    /// (day of week, hour, minute) seen by the last [`Self::tick`].
    last_minute: Option<(u8, u8, u8)>,
}

impl Default for RealTimeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl RealTimeClock {
    /// Create new RTC
    pub const fn new() -> Self {
        Self {
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
            internal_output: [0_u8; 7],
            command: 0,
            input: 0,
            input_bit_num: 0,
            input_index: 0,
            output_bit_num: 0,
            output_index: 0,
            fixed_time: None,
            time_offset: 0,
            clock_adjust: 0,
            free_reg: 0,
            irq_flag: 0,
            last_minute: None,
        }
    }

    /// Current local (or fixed) time.
    fn now(&self) -> chrono::NaiveDateTime {
        self.fixed_time.unwrap_or_else(|| {
            chrono::Local::now().naive_local() + chrono::TimeDelta::seconds(self.time_offset)
        })
    }

    /// Hour register value as read back: BCD, 12/24-hour mode, PM in bit 6.
    fn hour_reg(&self, hour: u8) -> u8 {
        let h = if self.stat1_reg & 0x2 != 0 { hour } else { hour % 12 }; // STAT1 bit 1: 24-hour mode
        byte_to_bcd(h) | (u8::from(hour >= 12) << 6) // PM flag (bit 6)
    }

    /// Whether `alarm` matches: each field with bit 7 set is compared.
    const fn alarm_matches(alarm: &Alarm, now: (u8, u8, u8)) -> bool {
        (alarm.day_of_week & 0x80 == 0 || alarm.day_of_week & 0x07 == now.0)
            && (alarm.hour & 0x80 == 0 || alarm.hour & 0x7F == now.1)
            && (alarm.minute & 0x80 == 0 || alarm.minute & 0x7F == now.2)
    }

    /// Checks the minute interrupts (melonDS `RTC::ProcessIRQ` type 0);
    /// call regularly (once per frame). Returns whether an interrupt line
    /// went active, which raises the ARM7 RTC IRQ when RCNT routes it.
    ///
    /// ```text
    ///  STAT2 bits 0-3 (INT1): 0010/0110/0011/0111 per-minute, 0100 alarm 1
    ///  STAT2 bit 6    (INT2): alarm 2
    /// ```
    pub fn tick(&mut self) -> bool {
        use chrono::{Datelike, Timelike};
        let t = self.now();
        let now = (
            t.weekday().num_days_from_sunday() as u8,
            self.hour_reg(t.hour() as u8),
            byte_to_bcd(t.minute() as u8),
        );
        if self.last_minute.replace(now).is_none_or(|last| last == now) {
            return false;
        }
        let old = self.irq_flag;
        match self.stat2_reg & 0x0F {
            0b0010 | 0b0110 | 0b0011 | 0b0111 => self.irq_flag |= 0x10,
            0b0100 => {
                if Self::alarm_matches(&self.alarm1, now) {
                    self.irq_flag |= 0x10;
                } else {
                    self.irq_flag &= !0x10;
                }
            }
            _ => {}
        }
        if self.stat2_reg & 0x40 != 0 {
            if Self::alarm_matches(&self.alarm2, now) {
                self.irq_flag |= 0x20;
            } else {
                self.irq_flag &= !0x20;
            }
        }
        // The status register 1 shows the active lines.
        self.stat1_reg |= self.irq_flag & 0x30;
        old & 0x30 == 0 && self.irq_flag & 0x30 != 0
    }

    /// Pins the clock to a fixed date/time (or `None` to follow the host's
    /// local time again).
    pub fn set_fixed_time(&mut self, time: Option<(i32, u32, u32, u32, u32, u32)>) {
        self.fixed_time = time.and_then(|(y, mo, d, h, mi, s)| {
            chrono::NaiveDate::from_ymd_opt(y, mo, d)?.and_hms_opt(h, mi, s)
        });
    }

    /// Runs the clock from `time` on (seconds offset from the host's local
    /// time).
    pub fn set_running_time(&mut self, time: (i32, u32, u32, u32, u32, u32)) {
        let (y, mo, d, h, mi, s) = time;
        if let Some(t) =
            chrono::NaiveDate::from_ymd_opt(y, mo, d).and_then(|d| d.and_hms_opt(h, mi, s))
        {
            self.fixed_time = None;
            self.time_offset = (t - chrono::Local::now().naive_local()).num_seconds();
        }
    }

    /// Initialize RTC with default values
    pub const fn init(&mut self) {
        // Initialize to default date/time (2000-01-01 00:00:00)

        // Place a hardcoded date into the RTC
        // For those curious, this is January 1st, 2005, a couple of months after the release of the NDS
        self.year = 0x05;
        // DS Real-Time Clock (RTC): https://problemkaputt.de/gbatek.htm#dsrealtimeclockrtc
        self.month = 0x1; // BCD month 01
        self.day = 0x1; // BCD day 01
        self.day_of_week = 0x0; // day of week 0

        // Set the time to midnight
        self.hour = 0x12;
        self.minute = 0x0; // BCD minute 00
        self.second = 0x0; // BCD second 00

        self.io_reg = 0;
        self.stat1_reg = 0;
        self.stat2_reg = 0;
    }

    /// Process input data from serial protocol
    pub fn interpret_input(&mut self) {
        #[cfg(feature = "log")]
        log::trace!(target: "rtc", "byte {} = {:02X}", self.input_index, self.input);
        if self.input_index == 0 {
            self.command = (self.input & 0x70) >> 4; // command byte: command (bits 4-6)

            if (self.input & 0x80) != 0 {
                // command byte bit 7: read
                match self.command {
                    0 => {
                        self.internal_output[0] = self.stat1_reg;
                        // reading status 1 acknowledges the interrupt flags
                        self.stat1_reg &= !0x30;
                        self.irq_flag = 0;
                    }
                    1 => {
                        if (self.stat2_reg & 0x4) != 0 {
                            // STAT2 bit 2: INT1 frequency/alarm mode
                            self.internal_output[0] = self.alarm1.day_of_week;
                            self.internal_output[1] = self.alarm1.hour;
                            self.internal_output[2] = self.alarm1.minute;
                        } else {
                            self.internal_output[0] = self.alarm1.minute;
                        }
                    }
                    2 => {
                        use chrono::{Datelike, Local, Timelike};

                        let now = self.fixed_time.unwrap_or_else(|| Local::now().naive_local());

                        // tm_year - 100
                        // C: tm_year = years since 1900
                        // e.g.: 2025 → 125 → 125 - 100 = 25
                        self.internal_output[0] = byte_to_bcd((now.year() - 2000) as u8);

                        // tm_mon + 1 (1 based index)
                        self.internal_output[1] = byte_to_bcd(now.month() as u8);

                        // tm_mday
                        self.internal_output[2] = byte_to_bcd(now.day() as u8);

                        // tm_wday (C: 0 = Sunday)
                        self.internal_output[3] =
                            byte_to_bcd(now.weekday().num_days_from_sunday() as u8);

                        // hour
                        let hour = now.hour() as u8;

                        if (self.stat1_reg & 0x2) > 0 {
                            // STAT1 bit 1: 24-hour mode
                            self.internal_output[4] = byte_to_bcd(hour); // 24 hour mode
                        } else {
                            self.internal_output[4] = byte_to_bcd(hour % 12); // 12 hour mode
                        }

                        self.internal_output[4] |= ((hour >= 12) as u8) << 6; // PM flag (bit 6)
                        self.internal_output[5] = byte_to_bcd(now.minute() as u8); // minute
                        self.internal_output[6] = byte_to_bcd(now.second() as u8); // second
                    }

                    3 => self.internal_output[0] = self.clock_adjust,
                    4 => self.internal_output[0] = self.stat2_reg,
                    7 => self.internal_output[0] = self.free_reg,
                    5 => {
                        self.internal_output[0] = self.alarm2.day_of_week;
                        self.internal_output[1] = self.alarm2.hour;
                        self.internal_output[2] = self.alarm2.minute;
                    }
                    6 => {
                        self.internal_output[0] = self.hour;
                        self.internal_output[1] = self.minute;
                        self.internal_output[2] = self.second;
                    }

                    _ => {
                        #[cfg(feature = "log")]
                        log::error!("Unrecognized read from RTC command {}", self.command);
                    }
                }
            }
        } else {
            match self.command {
                0 => {
                    self.stat1_reg = self.input as u8;
                }

                1 => {
                    if (self.stat2_reg & 0x4) != 0 {
                        // STAT2 bit 2: INT1 frequency/alarm mode
                        match self.input_index {
                            1 => self.alarm1.day_of_week = self.input as u8,
                            2 => self.alarm1.hour = self.input as u8,
                            3 => self.alarm1.minute = self.input as u8,
                            _ => {}
                        }
                    } else if self.input_index == 1 {
                        self.alarm1.minute = self.input as u8;
                    }
                }
                #[allow(clippy::match_same_arms)]
                2 => {} // do nothing for now
                3 if self.input_index == 1 => self.clock_adjust = self.input as u8,
                4 if self.input_index == 1 => {
                    self.stat2_reg = self.input as u8;
                    // switching INT1 / INT2 off releases their lines
                    if self.stat2_reg & 0x0F == 0 {
                        self.irq_flag &= !0x10;
                    }
                    if self.stat2_reg & 0x40 == 0 {
                        self.irq_flag &= !0x20;
                    }
                }
                7 if self.input_index == 1 => self.free_reg = self.input as u8,

                5 => match self.input_index {
                    1 => self.alarm2.day_of_week = self.input as u8,
                    2 => self.alarm2.hour = self.input as u8,
                    3 => self.alarm2.minute = self.input as u8,
                    _ => {}
                },
                _ => {
                    #[cfg(feature = "log")]
                    log::error!(
                        "Unrecognized write ${:02X} to RTC command {}",
                        self.input,
                        self.command
                    );
                }
            }
        }

        self.input_index += 1;
    }

    /// Read from RTC
    pub const fn read(&self) -> u16 {
        self.io_reg
    }

    /// Write to the RTC register.
    pub fn write(&mut self, mut value: u16, is_byte: bool) {
        if is_byte {
            value |= self.io_reg & 0xFF00; // byte write keeps the upper half of the register
        }

        // Extract control bits
        let data = (value & 0x0001) as u32; // Bit 0: data in/out
        let clock_out = (value & 0x0002) != 0; // Bit 1: clock (active low)
        let select_out = (value & 0x0004) != 0; // Bit 2: select (chip enable)
        let is_writing = (value & 0x0010) != 0; // Bit 4: direction (1 = write)

        if select_out && (self.io_reg & 0x0004) == 0 {
            // bit 2 (CS) going high starts a transfer
            self.input = 0;
            self.input_bit_num = 0;
            self.input_index = 0;
            self.output_bit_num = 0;
            self.output_index = 0;
        }

        // Chip select handling & If select changed from LOW -> HIGH, reset buffers
        // When select is already active and clock transitions to low
        if (select_out && !clock_out) && is_writing {
            // Writing data bit into RTC
            self.input |= data << self.input_bit_num;
            self.input_bit_num += 1;

            // When one byte is filled, interpret it
            if self.input_bit_num == 8 {
                self.input_bit_num = 0;
                self.interpret_input();
                self.input = 0;
            }
        }

        if (select_out && !clock_out) && !is_writing {
            if (self.internal_output[self.output_index as usize] & (1 << self.output_bit_num)) != 0
            {
                self.io_reg |= 0x0001; // set bit 0
            } else {
                self.io_reg &= !0x0001; // clear bit 0
            }
            self.output_bit_num += 1;

            // Advance to next output byte
            if self.output_bit_num == 8 {
                self.output_bit_num = 0;
                if self.output_index < 7 {
                    self.output_index += 1;
                }
            }
        }

        // Commit the written value to io_reg
        //
        // If writing mode: full register is updated.
        // If reading mode: only bit 0 comes from RTC, the rest from `value`.
        if is_writing {
            self.io_reg = value;
        } else {
            self.io_reg = (self.io_reg & 0x0001) | (value & 0xFE); // keep data bit 0, take the direction/clock/select bits
        }
    }
}
