/// Serial Peripheral Interface (SPI) bus for Nintendo DS
/// Manages communication with Firmware, Touchscreen, and other SPI devices

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
pub struct SPIBus {
    /// Emulator reference
    emulator_ptr: *mut crate::emulator::Emulator,

    /// Firmware device
    firmware_active: bool,
    /// Touchscreen device
    touchscreen_active: bool,

    /// SPI control register
    spicnt: RegSpiCnt,

    /// Output data buffer
    output: u8,
    /// Input data buffer
    input: u8,
}

impl SPIBus {
    /// Create new SPI bus
    pub fn new() -> Self {
        SPIBus {
            emulator_ptr: std::ptr::null_mut(),
            firmware_active: false,
            touchscreen_active: false,
            spicnt: RegSpiCnt::new(),
            output: 0,
            input: 0,
        }
    }

    /// Initialize SPI bus with firmware from file
    pub fn init(&mut self, _firmware_path: &str) -> Result<(), String> {
        self.firmware_active = true;
        Ok(())
    }

    /// Initialize SPI bus with firmware data
    pub fn init_with_firmware(&mut self, _firmware: &[u8]) -> Result<(), String> {
        self.firmware_active = true;
        Ok(())
    }

    /// Direct boot (skip firmware loading)
    pub fn direct_boot(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Handle touchscreen press
    pub fn touchscreen_press(&mut self, x: i32, y: i32) -> Result<(), String> {
        // Update touchscreen with press coordinates
        if self.spicnt.device == 1 {
            // Device 1 is touchscreen
            // Queue position data
        }
        Ok(())
    }

    /// Read from SPI data register
    pub fn read_spidata(&mut self) -> u8 {
        let data = self.output;

        // If transfer is complete, clear busy flag
        if !self.spicnt.transfer_16bit {
            self.spicnt.busy = false;

            // Trigger interrupt if enabled
            if self.spicnt.irq_after_transfer {
                // Request interrupt
            }
        }

        data
    }

    /// Write to SPI data register
    pub fn write_spidata(&mut self, data: u8) -> Result<(), String> {
        self.input = data;
        self.spicnt.busy = true;

        // Process transfer based on device selection
        match self.spicnt.device {
            0 => {
                // Firmware device
                self.firmware_transfer(data)?;
            }
            1 => {
                // Touchscreen device
                self.touchscreen_transfer(data)?;
            }
            2 => {
                // Power management device
                self.power_transfer(data)?;
            }
            3 => {
                // GBA cartridge (not typically used on DS)
            }
            _ => {}
        }

        Ok(())
    }

    /// Get SPI control register
    pub fn get_spicnt(&self) -> u16 {
        self.spicnt.get()
    }

    /// Set SPI control register
    pub fn set_spicnt(&mut self, value: u16) {
        self.spicnt.set(value);
    }

    // Private device transfer methods

    /// Handle firmware device transfer
    fn firmware_transfer(&mut self, _data: u8) -> Result<(), String> {
        // Firmware command processing
        // Commands: read page, write page, etc.
        Ok(())
    }

    /// Handle touchscreen device transfer
    fn touchscreen_transfer(&mut self, data: u8) -> Result<(), String> {
        // Touchscreen command processing
        // Commands: read X, read Y, read temperature, etc.
        // ADS7843 compatible protocol
        match (data >> 4) & 0x7 {
            0x0 => {
                // Measure Y position
            }
            0x1 => {
                // Measure X position
            }
            0x3 => {
                // Measure Z1 (pressure)
            }
            0x4 => {
                // Measure Z2 (pressure)
            }
            0x5 => {
                // Measure temperature 0
            }
            0x6 => {
                // Measure temperature 1
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle power management device transfer
    fn power_transfer(&mut self, _data: u8) -> Result<(), String> {
        // Power management command processing
        // Device can control backlight, hardware power, etc.
        Ok(())
    }

    /// Get bandwidth in MHz
    pub fn get_bandwidth_mhz(&self) -> u32 {
        match self.spicnt.bandwidth {
            0 => 4,
            1 => 2,
            2 => 1,
            3 => 0, // 512 kHz
            _ => 4,
        }
    }

    /// Get device name
    pub fn get_device_name(&self) -> &'static str {
        match self.spicnt.device {
            0 => "Firmware",
            1 => "Touchscreen",
            2 => "Power Management",
            3 => "GBA Cartridge",
            _ => "Unknown",
        }
    }
}

impl Default for SPIBus {
    fn default() -> Self {
        Self::new()
    }
}
