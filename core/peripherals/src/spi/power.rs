//! Power management IC (SPI device 0).
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
//!        ^^^ this file
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
//! The chip switches the system off, controls the backlights, the sound
//! amplifier and the microphone gain, and reports the battery state.
//! See GBATEK "DS Power Management".

/// Power management chip on SPI device 0 (melonDS `PowerMan`).
///
/// The first byte selects a register (bit 7 = read), the second byte is the
/// data. Registers: 0 control, 1 battery (0 = OK), 2 amplifier, 3 mic gain,
/// 4 backlight.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct PowerMan {
    selected: bool,
    pos: u8,
    index: u8,
    registers: [u8; 8],
}

impl Default for PowerMan {
    fn default() -> Self {
        let mut registers = [0; 8];
        // DS Power Management Device: https://problemkaputt.de/gbatek.htm#dspowermanagementdevice
        registers[4] = 0x40; // register 4: backlight level/charger default
        Self { selected: false, pos: 0, index: 0, registers }
    }
}

impl PowerMan {
    const MASKS: [u8; 8] = [0x7F, 0x00, 0x01, 0x03, 0x0F, 0, 0, 0]; // writable bits per power management register

    /// First byte selects a register (bit 7 = read), the second reads or writes it.
    pub const fn transfer(&mut self, value: u8) -> u8 {
        if !self.selected {
            self.selected = true;
            self.index = value;
            self.pos = 1;
            return 0;
        }
        let out = if self.pos == 1 {
            let reg = (self.index & 0x07) as usize; // register index (bits 0-2)
            if self.index & 0x80 != 0 {
                // index bit 7: read
                self.registers[reg]
            } else {
                let mask = Self::MASKS[reg];
                self.registers[reg] = (self.registers[reg] & !mask) | (value & mask);
                0
            }
        } else {
            0
        };
        self.pos = self.pos.saturating_add(1);
        out
    }

    /// Chip select released: the next byte is a register index again.
    pub const fn release(&mut self) {
        self.selected = false;
    }
}
