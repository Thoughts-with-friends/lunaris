//! Touch screen controller, TSC (SPI device 2).
//!
//! ```text
//!  ARM7 CPU
//!    |  SPICNT  (0x040001C0)  bits 8-9: device select, bit 11: chip-select hold
//!    |  SPIDATA (0x040001C2)  one byte out / one byte in per transfer
//!    v
//!  +--------------------------- SPI bus (spi/mod.rs) ---------------------------+
//!  |        device 0                device 1                   device 2          |
//!  +-----------|-----------------------|--------------------------|--------------+
//!              v                       v                          v
//!        power.rs                firmware.rs                touchscreen.rs
//!                                                           ^^^ this file
//!        power management IC     firmware flash memory      touch screen controller
//!        (power off, backlight,  (user settings, Wi-Fi      (TSC: pen position,
//!         amplifier, mic gain)    settings, boot code)       mic, temperature ADC)
//! ```
//!
//! None of these chips is mapped into the ARM7 address space: the CPU can
//! only talk to them by selecting one with SPICNT and then exchanging bytes
//! through SPIDATA. That is why the three device models live under `spi/`,
//! next to the bus that multiplexes them.
//!
//! The TSC is an ADC: a control byte selects a channel (touch X/Y, pressure,
//! microphone, temperature) and the next two bytes return the 12-bit
//! result. See GBATEK "DS Touch Screen Controller (TSC)".
//!
//! ```text
//!  channel  0 TEMP0   1 Y   2 battery   3 Z1   4 Z2   5 X   6 AUX (mic)   7 TEMP1
//!
//!  mic: the frontend hands over one video frame of samples
//!       (set_mic_frame); a read at cycle c of the frame returns
//!       sample[c * len / FRAME_CYCLES] as an unsigned 12-bit value
//!       (no samples: 800h, silence)
//! ```

/// Bus cycles in one video frame (263 lines of 2130 cycles).
pub const FRAME_CYCLES: u64 = 560_190;

/// Pressure readings while the pen is down (Z1 low, Z2 high = light
/// touch; GBATEK: R = Rx * X / 4096 * (Z2 / Z1 - 1)).
const PRESS_Z1: u16 = 0x300;
const PRESS_Z2: u16 = 0xA00;
/// Temperature diode readings around room temperature (TEMP1 - TEMP0 is
/// what software turns into degrees).
const TEMP0: u16 = 0x0960;
const TEMP1: u16 = 0x0A90;

/// Touch screen controller emulation.
///
/// This struct emulates the Nintendo DS touchscreen SPI device.
/// It faithfully reproduces the behavior of the original C++
/// implementation from CorgiDS.
#[derive(serde::Serialize, serde::Deserialize, Debug, Default)]
pub struct TouchScreen {
    // Control byte written by SPI
    control_byte: u8,

    // Output coordinate buffer (12-bit ADC result)
    output_coords: u16,

    // Current byte position during SPI transfer
    data_pos: i32,

    // Latched touch coordinates
    press_x: u16,
    press_y: u16,

    /// Microphone samples for the current frame (host data).
    #[serde(skip)]
    mic: Vec<i16>,
    /// Bus cycles since the start of the frame, set before each transfer.
    #[serde(skip)]
    frame_pos: u64,
}

impl TouchScreen {
    /// Create a new TouchScreen instance.
    ///
    /// Initializes all internal state exactly as the original
    /// C++ constructor.
    pub const fn new() -> Self {
        Self {
            data_pos: 0,
            control_byte: 0,
            output_coords: 0,
            press_x: 0,
            // DS Touch Screen Controller (TSC): https://problemkaputt.de/gbatek.htm#dstouchscreencontrollertsc
            press_y: 0xFFF, // 12-bit ADC "not pressed" value
            mic: Vec::new(),
            frame_pos: 0,
        }
    }

    /// Microphone input for the next frame (signed 16-bit mono, any
    /// sample rate: the samples are spread over the frame). An empty
    /// slice is silence.
    pub fn set_mic_frame(&mut self, samples: &[i16]) {
        self.mic.clear();
        self.mic.extend_from_slice(samples);
    }

    /// Bus cycles since the start of the current frame.
    pub const fn set_frame_pos(&mut self, cycles: u64) {
        self.frame_pos = cycles;
    }

    /// AUX channel: the mic sample at this point of the frame, unsigned
    /// 12-bit (800h = silence).
    fn mic_sample(&self) -> u16 {
        if self.mic.is_empty() {
            return 0x800;
        }
        let pos = self.frame_pos.min(FRAME_CYCLES - 1);
        let index = (pos * self.mic.len() as u64 / FRAME_CYCLES) as usize;
        ((self.mic[index] as u16) ^ 0x8000) >> 4 // signed 16-bit -> unsigned 12-bit
    }

    const fn pressed(&self) -> bool {
        self.press_y != 0xFFF
    }

    /// Power on (or reset) the touchscreen device.
    ///
    /// This resets all internal state and latched coordinates,
    /// matching the original C++ `power_on()` behavior.
    pub const fn power_on(&mut self) {
        self.data_pos = 0;
        self.control_byte = 0;
        self.output_coords = 0;

        self.press_x = 0;
        self.press_y = 0xFFF; // 12-bit ADC "not pressed" value
    }

    /// Register a touch press event.
    ///
    /// The coordinates are latched internally and converted
    /// to 12-bit ADC values by shifting left by 4 bits,
    /// exactly like the original implementation.
    ///
    /// A Y value of `0xFFF` indicates "no touch".
    pub const fn press_event(&mut self, x: i32, y: i32) {
        self.press_x = x as u16;
        self.press_y = y as u16;

        if y == 0xFFF {
            // not pressed
            return;
        }

        self.press_x <<= 4;
        self.press_y <<= 4;

        // printf("\nTouchscreen: ($%04X, $%04X)", press_x, press_y);
    }

    /// Transfer one byte over the touchscreen SPI interface.
    ///
    /// This function emulates the exact SPI behavior of the
    /// touchscreen controller, including:
    /// - Control byte decoding
    /// - Channel selection
    /// - ADC result shifting
    /// - Data position tracking
    pub fn transfer_data(&mut self, input: u8) -> u8 {
        // Output previously prepared data
        let data = if self.data_pos == 0 {
            ((self.output_coords >> 5) & 0xFF) as u8 // first byte: ADC bits 5-11
        } else if self.data_pos == 1 {
            ((self.output_coords << 3) & 0xFF) as u8 // second byte: ADC bits 0-4 in bits 3-7
        } else {
            0
        };

        if (input & (1 << 7)) != 0 {
            // Set control byte
            self.control_byte = input;
            let channel = (self.control_byte >> 4) & 0x7; // control byte channel select (bits 4-6)
            self.data_pos = 0;

            // Select ADC channel
            match channel {
                0 => self.output_coords = TEMP0,
                1 => self.output_coords = self.press_y, // Touch Y
                3 => self.output_coords = if self.pressed() { PRESS_Z1 } else { 0 },
                4 => self.output_coords = if self.pressed() { PRESS_Z2 } else { 0xFFF },
                5 => self.output_coords = self.press_x, // Touch X
                6 => self.output_coords = self.mic_sample(), // AUX: microphone
                7 => self.output_coords = TEMP1,
                _ => self.output_coords = 0xFFF, // battery (unused on the DS) reads all ones
            }

            // Conversion mode change
            if (self.control_byte & 0x8) != 0 {
                self.output_coords &= 0x0FF0; // 8-bit conversion mode keeps the upper 8 bits
            }
        } else {
            self.data_pos += 1;
        }

        data
    }

    /// Deselect the touchscreen SPI device.
    ///
    /// This resets the internal transfer position, exactly
    /// like the original C++ `deselect()` method.
    pub const fn deselect(&mut self) {
        self.data_pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Selects `channel` (12-bit mode) and reads the two result bytes.
    fn read(tsc: &mut TouchScreen, channel: u8) -> u16 {
        tsc.transfer_data(0x80 | (channel << 4));
        let hi = u16::from(tsc.transfer_data(0));
        let lo = u16::from(tsc.transfer_data(0));
        (hi << 5) | (lo >> 3)
    }

    #[test]
    fn mic_follows_the_frame_position() {
        let mut tsc = TouchScreen::new();
        assert_eq!(read(&mut tsc, 6), 0x800, "no input: silence");
        tsc.set_mic_frame(&[i16::MIN, 0, i16::MAX, 0]);
        tsc.set_frame_pos(0);
        assert_eq!(read(&mut tsc, 6), 0x000);
        tsc.set_frame_pos(FRAME_CYCLES / 2);
        assert_eq!(read(&mut tsc, 6), 0xFFF);
    }

    #[test]
    fn pressure_reports_pen_down() {
        let mut tsc = TouchScreen::new();
        assert_eq!(read(&mut tsc, 3), 0);
        tsc.press_event(100, 50);
        assert_eq!(read(&mut tsc, 3), PRESS_Z1);
        assert_eq!(read(&mut tsc, 4), PRESS_Z2);
        assert_eq!(read(&mut tsc, 5), 100 << 4);
    }
}
