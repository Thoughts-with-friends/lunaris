//! Nintendo DS Firmware controller
//! Handles firmware data loading, CRC verification, and SPI data transfer
use std::{fs::File, io::Read as _};

use crate::error::{EmuError, FailedReadFileSnafu};
use snafu::ResultExt as _;

/// Firmware commands
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareCommand {
    /// No command
    #[default]
    None = 0,
    /// Read status register
    ReadStatusReg = 1,
    /// Read data stream
    ReadStream = 2,
}

impl FirmwareCommand {
    /// Convert numeric value to FirmwareCommand
    pub fn from_value(val: u32) -> Self {
        match val {
            0 => FirmwareCommand::None,
            1 => FirmwareCommand::ReadStatusReg,
            2 => FirmwareCommand::ReadStream,
            _ => FirmwareCommand::None,
        }
    }
}

/// Nintendo DS Firmware
/// Stores firmware data and manages SPI communication
#[derive(Debug)]
pub struct Firmware {
    /// Firmware data (262 KB)
    pub(crate) raw_firmware: Vec<u8>,
    /// Status register
    status_reg: u8,
    /// User data section
    pub(crate) user_data: i32,

    /// Current command
    command_id: FirmwareCommand,
    /// Current address
    address: u32,
    /// Total arguments for command
    total_args: i32,
}

impl Firmware {
    /// Firmware size in bytes (256 KB)
    pub const SIZE: usize = 1024 * 256;

    /// Create new Firmware controller
    pub fn new() -> Self {
        Firmware {
            raw_firmware: vec![0u8; Self::SIZE],
            status_reg: 0,
            user_data: 0,
            command_id: FirmwareCommand::None,
            address: 0,
            total_args: 0,
        }
    }

    /// Load firmware from file.
    ///
    /// This function faithfully mirrors the original C++ implementation:
    /// - Attempts to load firmware from a binary file
    /// - Falls back to default firmware if the file cannot be opened
    /// - Performs user data selection and CRC verification
    /// - Patches several firmware configuration fields
    /// - Recalculates CRCs for user data and header
    pub fn load_firmware(&mut self, file_name: &str) -> Result<usize, EmuError> {
        // Ensure firmware buffer has the correct size
        if self.raw_firmware.len() != Self::SIZE {
            self.raw_firmware.resize(Self::SIZE, 0);
        }

        // Try to open firmware file
        let mut firmware_file = File::open(file_name).ok();

        if firmware_file.is_none() {
            // Load default firmware if file open fails
            let fw = lunaris_ds_free_bios::firmware::FIRMWARE_DS;

            // Copy default firmware into buffer
            let copy_size = fw.len().min(Self::SIZE);
            self.raw_firmware[..copy_size].copy_from_slice(&fw[..copy_size]);

            #[cfg(feature = "tracing")]
            tracing::info!("Loaded free firmware.");
        } else {
            // Read firmware file directly into buffer (no bounds checking, same as C++)
            let mut file = firmware_file.take().unwrap();
            file.read_exact(&mut self.raw_firmware)
                .with_context(|_| FailedReadFileSnafu { path: file_name })?;
        }

        // Initial user data base address
        self.user_data = 0x3FE00;

        // Read USER1 sequence number and compare against USER0
        let user0_seq = self.read_u16((self.user_data + 0x70) as usize);
        let user1_seq = self.read_u16((self.user_data + 0x170) as usize);

        if user1_seq == ((user0_seq + 1) & 0x7F) {
            // Verify CRC of USER1 data
            #[rustfmt::skip]
            let verify = self.verify_crc(0xFFFF, (self.user_data + 0x100) as usize, 0x70, (self.user_data + 0x172) as usize);
            if verify {
                // Switch to USER1 block
                self.user_data += 0x100;
            }
        }

        // Patch user configuration fields (exact offsets preserved)
        self.write_u16((self.user_data + 0x58) as usize, 0);
        self.write_u16((self.user_data + 0x5A) as usize, 0);
        self.raw_firmware[(self.user_data + 0x5C) as usize] = 0;
        self.raw_firmware[(self.user_data + 0x5D) as usize] = 0;

        self.write_u16((self.user_data + 0x5E) as usize, 255 << 4);
        self.write_u16((self.user_data + 0x60) as usize, 191 << 4);
        self.raw_firmware[(self.user_data + 0x62) as usize] = 255;
        self.raw_firmware[(self.user_data + 0x63) as usize] = 191;

        // Recalculate USER data CRC
        let user_crc =
            Self::create_crc(&self.raw_firmware[self.user_data as usize..], 0x70, 0xFFFF);
        self.write_u16((self.user_data + 0x72) as usize, user_crc);

        // Recalculate firmware header CRC
        let header_len = self.read_u16(0x2C) as usize;
        let header_crc = Self::create_crc(&self.raw_firmware[0x2C..], header_len, 0x0000);
        self.write_u16(0x2A, header_crc);

        // Debug output of CRC verification results
        #[cfg(feature = "tracing")]
        {
            tracing::error!(
                "\nFW: USER0 CRC16 = {}",
                if self.verify_crc(0xFFFF, 0x3FE00, 0x70, 0x3FE72) {
                    "GOOD"
                } else {
                    "BAD"
                },
            );
            tracing::error!(
                "FW: USER1 CRC16 = {}",
                if self.verify_crc(0xFFFF, 0x3FF00, 0x70, 0x3FF72) {
                    "GOOD"
                } else {
                    "BAD"
                },
            );
        }

        // Reset command and status registers
        self.command_id = FirmwareCommand::None;
        self.status_reg = 0;

        // Always return 0 in C++ version; here we return loaded size
        Ok(Self::SIZE)
    }

    /// Read a little-endian u16 from firmware
    fn read_u16(&self, offset: usize) -> u16 {
        u16::from_le_bytes([self.raw_firmware[offset], self.raw_firmware[offset + 1]])
    }

    /// Write a little-endian u16 into firmware
    fn write_u16(&mut self, offset: usize, value: u16) {
        let bytes = value.to_le_bytes();
        self.raw_firmware[offset] = bytes[0];
        self.raw_firmware[offset + 1] = bytes[1];
    }

    /// Create a CRC16 value from the given data buffer.
    ///
    /// This is a faithful Rust translation of the original
    /// C++ `Firmware::create_CRC` implementation.
    /// All bit operations, constants, and control flow
    /// are preserved exactly.
    pub fn create_crc(data: &[u8], length: usize, mut start: u32) -> u16 {
        // CRC polynomial lookup table (identical to C++ code)
        let stuff: [u16; 8] = [
            0xC0C1, 0xC181, 0xC301, 0xC601, 0xCC01, 0xD801, 0xF001, 0xA001,
        ];

        // Process each byte
        #[expect(clippy::needless_range_loop)]
        for i in 0..length {
            start ^= data[i] as u32;

            // Process each bit
            for (j, &v) in stuff.iter().enumerate() {
                if (start & 0x1) != 0 {
                    start >>= 1;
                    start ^= (v as u32) << (7 - j);
                } else {
                    start >>= 1;
                }
            }
        }

        // Return lower 16 bits (equivalent to uint16_t cast)
        (start & 0xFFFF) as u16
    }

    /// Verify a CRC16 value stored in firmware against a calculated CRC.
    ///
    /// This function mirrors the original C++ `Firmware::verify_CRC`
    /// implementation exactly, including:
    /// - Little-endian uint16_t access
    /// - Debug output formatting
    /// - CRC comparison logic
    pub fn verify_crc(&self, start: u32, offset: usize, length: usize, crc_offset: usize) -> bool {
        // Read stored CRC (little-endian)
        let stored_crc = u16::from_le_bytes([
            self.raw_firmware[crc_offset],
            self.raw_firmware[crc_offset + 1],
        ]);

        // Calculate CRC from firmware data
        let calculated_crc = Firmware::create_crc(&self.raw_firmware[offset..], length, start);

        // Debug output (matches C++ printf behavior)
        // println!("\nStored CRC: ${:04X}", stored_crc);
        // println!("Calc CRC: ${:04X}", calculated_crc);

        stored_crc == calculated_crc
    }

    /// Transfer data byte via SPI
    /// Input: byte to send to firmware
    /// Returns: byte received from firmware
    pub fn transfer_data(&mut self, input: u8) -> u8 {
        match self.command_id {
            FirmwareCommand::None => {
                // Parse command byte
                self.command_id = FirmwareCommand::from_value(input as u32);
                self.total_args = 0;
                self.address = 0;
                0x00
            }
            FirmwareCommand::ReadStatusReg => {
                // Return status register
                self.command_id = FirmwareCommand::None;
                self.status_reg
            }
            FirmwareCommand::ReadStream => {
                // Return firmware data byte
                if (self.address as usize) < self.raw_firmware.len() {
                    let byte = self.raw_firmware[self.address as usize];
                    self.address = self.address.wrapping_add(1);
                    byte
                } else {
                    0x00
                }
            }
        }
    }

    /// Deselect firmware (end SPI transfer)
    #[expect(unused)]
    pub fn deselect(&mut self) {
        self.command_id = FirmwareCommand::None;
        self.address = 0;
        self.total_args = 0;
    }
}

impl Default for Firmware {
    fn default() -> Self {
        Self::new()
    }
}
