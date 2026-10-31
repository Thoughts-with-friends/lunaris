//! The ARM7's SPI bus and the three devices attached to it.
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
//! This module models the bus itself: the SPICNT/SPIDATA registers and the
//! routing of each transferred byte to the selected device. A device stays
//! selected across bytes while SPICNT bit 11 is set; clearing it ends the
//! current command. See GBATEK "DS Serial Peripheral Interface Bus (SPI)".

pub mod firmware;
pub mod power;
pub mod touchscreen;

use firmware::Firmware;
use power::PowerMan;
use touchscreen::TouchScreen;

/// SPI Control Register
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct RegSpiCnt {
    /// Baud rate (0=4MHz, 1=2MHz, 2=1MHz, 3=512kHz)
    pub bandwidth: u32,
    /// Transfer in progress
    pub busy: bool,
    /// Device select (0 = power management, 1 = firmware, 2 = touchscreen, 3 = reserved)
    pub device: u32,
    /// Transfer 16-bit instead of 8-bit
    pub transfer_16bit: bool,
    /// Keep chip select active after transfer
    pub chipselect_hold: bool,
    /// Generate interrupt after transfer
    pub irq_after_transfer: bool,
    /// SPI bus enabled
    pub enabled: bool,
}

impl RegSpiCnt {
    /// Create new SPI control register
    pub const fn new() -> Self {
        Self {
            bandwidth: 0,
            busy: false,
            device: 0,
            transfer_16bit: false,
            chipselect_hold: false,
            irq_after_transfer: false,
            enabled: false,
        }
    }

    /// Get register value as 16-bit halfword
    pub const fn get(&self) -> u16 {
        let mut value = 0_u16;
        // DS Serial Peripheral Interface Bus (SPI): https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi
        value |= (self.bandwidth & 0x3) as u16; // SPICNT baudrate (bits 0-1)
        if self.busy {
            // DS Serial Peripheral Interface Bus (SPI): https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi
            value |= 1 << 7; // SPICNT bit 7: busy
        }
        value |= ((self.device & 0x3) as u16) << 8; // SPICNT device select (bits 8-9)
        if self.transfer_16bit {
            value |= 1 << 10; // SPICNT bit 10: transfer size (16-bit)
        }
        if self.chipselect_hold {
            value |= 1 << 11; // SPICNT bit 11: chipselect hold
        }
        if self.irq_after_transfer {
            value |= 1 << 14; // SPICNT bit 14: interrupt request enable
        }
        if self.enabled {
            value |= 1 << 15; // SPICNT bit 15: SPI bus enable
        }
        value
    }

    /// Set register value from 16-bit halfword
    pub const fn set(&mut self, value: u16) {
        self.bandwidth = (value & 0x3) as u32; // SPICNT baudrate (bits 0-1)
        self.device = ((value >> 8) & 0x3) as u32; // SPICNT device select (bits 8-9)
        // DS Serial Peripheral Interface Bus (SPI): https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi
        self.transfer_16bit = (value & (1 << 10)) != 0; // SPICNT bit 10: transfer size (16-bit)
        self.chipselect_hold = (value & (1 << 11)) != 0; // SPICNT bit 11: chipselect hold
        self.irq_after_transfer = (value & (1 << 14)) != 0; // SPICNT bit 14: interrupt request enable
        self.enabled = (value & (1 << 15)) != 0; // SPICNT bit 15: SPI bus enable
    }
}

impl Default for RegSpiCnt {
    fn default() -> Self {
        Self::new()
    }
}

/// SPI Bus controller
/// Manages communication with multiple SPI-attached devices
#[derive(serde::Serialize, serde::Deserialize, Debug, Default)]
pub struct SPIBus {
    /// Emulator reference
    // pub(crate) emulator: EmulatorRef,
    pub firmware: Firmware,
    touchscreen: TouchScreen,
    powerman: PowerMan,

    /// SPI control register
    spicnt: RegSpiCnt,

    /// Output data buffer
    output: u8,
}

impl SPIBus {
    /// Create new SPI bus
    pub fn new() -> Self {
        Self {
            firmware: Firmware::new(),
            touchscreen: TouchScreen::new(),
            powerman: PowerMan::default(),
            spicnt: RegSpiCnt::new(),
            output: 0,
        }
    }

    /// Initializes the bus with a firmware image (see [`Firmware::load`]).
    pub fn init(&mut self, firmware_image: &[u8]) {
        self.firmware.load(firmware_image);

        self.spicnt.busy = false;
        self.spicnt.enabled = false;
        self.touchscreen.power_on();
    }

    /// Microphone samples for the next frame (see
    /// [`TouchScreen::set_mic_frame`]).
    pub fn set_mic_frame(&mut self, samples: &[i16]) {
        self.touchscreen.set_mic_frame(samples);
    }

    /// Bus cycles since the start of the frame, for the mic position.
    pub const fn set_frame_pos(&mut self, cycles: u64) {
        self.touchscreen.set_frame_pos(cycles);
    }

    /// Power management chip: battery low indicator (register 1 bit 0).
    pub const fn set_battery_low(&mut self, low: bool) {
        self.powerman.set_battery_low(low);
    }

    pub const fn battery_low(&self) -> bool {
        self.powerman.battery_low()
    }

    /// Power management chip: power off requested (register 0 bit 6).
    pub const fn power_off_requested(&self) -> bool {
        self.powerman.power_off()
    }

    /// Power management chip: (top, bottom) backlights on.
    pub const fn backlights(&self) -> (bool, bool) {
        self.powerman.backlights()
    }

    /// Latches the stylus position in the TSC (from ds_emu touchscreen_press).
    pub const fn touchscreen_press(&mut self, x: i32, y: i32) {
        self.touchscreen.press_event(x, y);
    }

    /// Read from SPI data register (0 while the bus is disabled).
    pub const fn read_spidata(&self) -> u8 {
        if self.spicnt.enabled { self.output } else { 0 }
    }

    /// Write to SPI data register: clocks one byte through the selected
    /// device. When SPICNT bit 11 (chip select hold) is clear the device is
    /// released afterwards, ending the current command.
    ///
    /// ```text
    /// from: ds_emu arm7_write_halfword(0x040001C2) (ARM7 only)
    ///   |
    ///   v
    /// write_spidata(byte)
    ///   bus disabled -> ignore
    ///   SPICNT device:
    ///     0 power manager -> powerman.transfer()   (backlight, amp...)
    ///     1 firmware      -> firmware.transfer_data() (user settings,
    ///                        Wi-Fi calibration, NVRAM writes)
    ///     2 touch screen  -> touchscreen.transfer_data() (pen x/y ADC)
    ///   no chip-select hold -> release / deselect the device
    ///   result byte -> `output` (read back at 0x040001C2)
    ///   |
    ///   v
    /// to:   caller raises the SPI IRQ when SPICNT bit 14 asks for it
    /// ```
    ///
    /// Returns `true` if the SPI transfer-complete IRQ must be raised.
    pub fn write_spidata(&mut self, data: u8) -> bool {
        if !self.spicnt.enabled {
            return false;
        }
        self.spicnt.busy = false;

        let hold = self.spicnt.chipselect_hold;
        self.output = match self.spicnt.device {
            1 => {
                let out = self.firmware.transfer_data(data);
                if !hold {
                    self.firmware.release();
                }
                out
            }
            2 => {
                let out = self.touchscreen.transfer_data(data);
                if !hold {
                    self.touchscreen.deselect();
                }
                out
            }
            0 => {
                let out = self.powerman.transfer(data);
                if !hold {
                    self.powerman.release();
                }
                out
            }
            _ => 0,
        };

        #[cfg(feature = "log")]
        log::trace!(
            target: "spi",
            "dev{} hold={hold} in={data:02X} out={:02X}",
            self.spicnt.device,
            self.output
        );

        self.spicnt.irq_after_transfer
    }

    /// Get SPI control register
    pub const fn get_spicnt(&self) -> u16 {
        self.spicnt.get()
    }

    /// Set SPI control register
    pub const fn set_spicnt(&mut self, value: u16) {
        self.spicnt.set(value);
    }
}
