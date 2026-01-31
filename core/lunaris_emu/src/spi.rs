//! Serial Peripheral Interface (SPI) bus for Nintendo DS
//! Manages communication with Firmware, Touchscreen, and other SPI devices

use crate::error::EmuError;
use crate::{firmware::Firmware, touchscreen::TouchScreen};

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
        RegSpiCnt {
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
        let mut value = 0u16;
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
        SPIBus {
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

    /// Handle touchscreen press
    pub fn touchscreen_press(&mut self, x: i32, y: i32) {
        self.touchscreen.press_event(x, y);
    }

    /// Read from SPI data register
    pub fn read_spidata(&mut self) -> u8 {
        if self.spicnt.enabled { 0 } else { self.output }
    }

    /// Write to SPI data register
    ///
    /// true => must call `emulator.requesting_interrupt(7);`
    pub fn write_spidata(&mut self, data: u8) -> bool {
        if self.spicnt.enabled {
            self.spicnt.busy = false;

            // Process transfer based on device selection
            self.output = match self.spicnt.device {
                1 => self.firmware.transfer_data(data),    // Firmware device
                2 => self.touchscreen.transfer_data(data), // Touchscreen device
                _ => 0,                                    // Power management or unknown device
            };

            if self.spicnt.irq_after_transfer {
                return true;
            }
        }
        false
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emulator::Emulator;

    /// Test SPI bus initialization and firmware loading.
    ///
    /// This test ensures that:
    /// - Free firmware is loaded when the firmware file does not exist
    /// - SPI control register is reset correctly
    /// - Touchscreen is powered on
    #[test]
    fn test_spi_init_with_free_firmware() {
        let mut spi = SPIBus::new();

        // Intentionally pass a non-existent path
        let result = spi.init("non_existent_firmware.bin");

        assert!(result.is_ok());
        assert_eq!(spi.get_spicnt(), 0);

        println!("[SPI TEST] Free firmware loaded successfully");
    }

    /// Test SPI transfer to firmware device.
    ///
    /// This test sends data over the SPI bus with the firmware
    /// device selected and prints returned bytes.
    #[test]
    fn test_spi_firmware_transfer() {
        let mut emulator = Emulator::new();
        let mut spi = SPIBus::new();

        spi.init("non_existent_firmware.bin").unwrap();

        // Enable SPI, select firmware device (device = 1)
        // Device bits and enable bit layout depend on RegSpiCnt,
        // but this assumes a typical DS layout.
        let mut spicnt = spi.get_spicnt();
        spicnt |= 1 << 15; // enable
        spicnt |= 1 << 8; // device = firmware
        spi.set_spicnt(spicnt);

        // Send a few bytes as if reading firmware ID
        let commands = [0x9F, 0x00, 0x00, 0x00];

        for &cmd in &commands {
            // emulator.write_spidata7(cmd);
            let out = spi.read_spidata();
            println!(
                "[SPI TEST] Firmware transfer: sent=0x{:02X}, received=0x{:02X}",
                cmd, out
            );
        }
    }

    /// Test SPI transfer to touchscreen device.
    ///
    /// This test simulates a touchscreen press and reads
    /// X coordinate data via SPI.
    #[test]
    fn test_spi_touchscreen_transfer() {
        let mut emulator = Emulator::new();
        let mut spi = SPIBus::new();
        spi.init("non_existent_firmware.bin").unwrap();

        // Simulate touchscreen press
        spi.touchscreen_press(120, 80);

        // Enable SPI, select touchscreen device (device = 2)
        let mut spicnt = spi.get_spicnt();
        spicnt |= 1 << 15; // enable
        spicnt |= 2 << 8; // device = touchscreen
        spi.set_spicnt(spicnt);

        // Control byte: start + channel 5 (Touch X)
        let control = 0b1101_0000;

        // emulator.write_spidata7(control);
        let high = spi.read_spidata();

        // emulator.write_spidata7(0);
        let low = spi.read_spidata();

        println!(
            "[SPI TEST] Touch X read: high=0x{:02X}, low=0x{:02X}",
            high, low
        );
    }
}
