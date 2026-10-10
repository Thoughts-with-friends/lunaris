//! Serial Peripheral Interface (SPI) bus for Nintendo DS
//! Manages communication with Firmware, Touchscreen, and other SPI devices

use crate::error::EmuError;
use crate::firmware::Firmware;
use lunaris_ds_touchscreen::TouchScreen;

/// SPI Control Register
#[derive(Debug, Clone, Copy)]
pub struct RegSpiCnt {
    /// Baud rate (0=4MHz, 1=2MHz, 2=1MHz, 3=512kHz)
    pub bandwidth: u32,
    /// Transfer in progress
    pub busy: bool,
    /// Device select (0=Firmware, 1=Touchscreen, 2=Power management, 3=GBA cartridge)
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
    pub fn new() -> Self {
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
    pub fn get(&self) -> u16 {
        let mut value = 0_u16;
        value |= (self.bandwidth & 0x3) as u16;
        if self.busy {
            value |= 1 << 7;
        }
        value |= ((self.device & 0x3) as u16) << 8;
        if self.transfer_16bit {
            value |= 1 << 10;
        }
        if self.chipselect_hold {
            value |= 1 << 11;
        }
        if self.irq_after_transfer {
            value |= 1 << 14;
        }
        if self.enabled {
            value |= 1 << 15;
        }
        value
    }

    /// Set register value from 16-bit halfword
    pub fn set(&mut self, value: u16) {
        self.bandwidth = (value & 0x3) as u32;
        self.device = ((value >> 8) & 0x3) as u32;
        self.transfer_16bit = (value & (1 << 10)) != 0;
        self.chipselect_hold = (value & (1 << 11)) != 0;
        self.irq_after_transfer = (value & (1 << 14)) != 0;
        self.enabled = (value & (1 << 15)) != 0;
    }
}

impl Default for RegSpiCnt {
    fn default() -> Self {
        Self::new()
    }
}

/// SPI Bus controller
/// Manages communication with multiple SPI-attached devices
#[derive(Debug, Default)]
pub struct SPIBus {
    /// Emulator reference
    // pub(crate) emulator: EmulatorRef,
    pub(crate) firmware: Firmware,
    touchscreen: TouchScreen,

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
            spicnt: RegSpiCnt::new(),
            output: 0,
        }
    }

    // NOTE C++ unused `init(uint_8 *firmware)`, Therefore, unimplemented here.
    /// Initialize SPI bus with firmware from file
    pub fn init(&mut self, firmware_path: &str) -> Result<(), EmuError> {
        self.firmware.load_firmware(firmware_path)?;

        self.spicnt.busy = false;
        self.spicnt.enabled = false;
        self.touchscreen.power_on();

        Ok(())
    }

    pub fn init_data(&mut self, firmware: Vec<u8>) -> Result<(), EmuError> {
        let _ = firmware;
        self.spicnt.busy = false;
        self.spicnt.enabled = false;
        self.touchscreen.power_on();
        Ok(())
    }

    pub fn touchscreen_press(&mut self, x: i32, y: i32) {
        self.touchscreen.press_event(x, y);
    }

    /// Read from SPI data register (0 while the bus is disabled).
    pub fn read_spidata(&self) -> u8 {
        if self.spicnt.enabled { self.output } else { 0 }
    }

    /// Write to SPI data register: clocks one byte through the selected
    /// device. When SPICNT bit 11 (chip select hold) is clear the device is
    /// released afterwards, ending the current command.
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
            _ => 0, // power management: not emulated
        };

        self.spicnt.irq_after_transfer
    }

    /// Get SPI control register
    pub fn get_spicnt(&self) -> u16 {
        self.spicnt.get()
    }

    /// Set SPI control register
    pub fn set_spicnt(&mut self, value: u16) {
        self.spicnt.set(value);
    }
}
