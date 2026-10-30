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
        }
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
    pub const fn transfer_data(&mut self, input: u8) -> u8 {
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
                1 => self.output_coords = self.press_y, // Touch Y
                5 => self.output_coords = self.press_x, // Touch X
                6 => self.output_coords = 0x800,        // Battery / auxiliary channel
                _ => self.output_coords = 0xFFF,        // unsupported channel reads all ones
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
