//! Card protocol on the ROM bus: the 8-byte command buffer (ROMCMD),
//! ROMCTRL and the data port. Commands are KEY1-encrypted during the
//! secure area phase ([`crate::key1`]). See GBATEK "DS Cartridge Protocol".
use crate::{NDSCart, NandCommand};

/// Cartridge command types
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CartCommand {
    /// No command
    Empty = 0,
    /// Dummy/NOP command
    Dummy = 1,
    /// Get cartridge header
    GetHeader = 2,
    /// Get chip ID
    GetChipId = 3,
    /// Enable KEY1 encryption
    EnableKey1 = 4,
    /// Enable KEY2 encryption
    EnableKey2 = 5,
    /// Get secure area block
    GetSecureAreaBlock = 6,
    /// Read ROM data
    ReadRom = 7,
    /// NAND save command (KEY2 mode, NAND cards: backup/nand.rs).
    Nand = 8,
    /// SD sector read / write (homebrew DLDI, sd.rs).
    Sd = 9,
}

impl CartCommand {
    /// Convert numeric value to CartCommand
    pub const fn from_value(val: u32) -> Self {
        match val {
            1 => Self::Dummy,
            2 => Self::GetHeader,
            3 => Self::GetChipId,
            4 => Self::EnableKey1,
            5 => Self::EnableKey2,
            6 => Self::GetSecureAreaBlock,
            7 => Self::ReadRom,
            8 => Self::Nand,
            9 => Self::Sd,
            _ => Self::Empty,
        }
    }
}

/// ROM control register
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct RegRomCtrl {
    /// KEY1 gap clocks (timing parameter)
    pub key1_gap: u32,
    /// KEY2 data enabled
    pub key2_data_enabled: bool,
    /// Apply KEY2 seed
    pub key2_apply_seed: bool,
    /// KEY2 gap setting
    pub key2_gap: u32,
    /// KEY2 command enabled
    pub key2_cmd_enabled: bool,
    /// Word ready signal
    pub word_ready: bool,
    /// Block size (transfer size)
    pub block_size: u32,
    /// Use slow transfer timing
    pub slow_transfer: bool,
    /// KEY1 gap uses clock cycles
    pub key1_gap_clocks: bool,
    /// Block transfer in progress
    pub block_busy: bool,
    /// ROMCTRL bit 30: data direction (1 = the CPU writes 0x04100010).
    pub write_dir: bool,
}

impl RegRomCtrl {
    /// Create new ROM control register
    pub const fn new() -> Self {
        Self {
            key1_gap: 0,
            key2_data_enabled: false,
            key2_apply_seed: false,
            key2_gap: 0,
            key2_cmd_enabled: false,
            word_ready: false,
            block_size: 0,
            slow_transfer: false,
            key1_gap_clocks: false,
            block_busy: false,
            write_dir: false,
        }
    }

    /// Get register value as 32-bit word
    pub const fn get(&self) -> u32 {
        let mut value = 0;
        value |= self.key1_gap;
        if self.key2_data_enabled {
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            value |= 1 << 13; // ROMCTRL bit 13: KEY2 encrypt data
        }
        if self.key2_apply_seed {
            value |= 1 << 15; // ROMCTRL bit 15: KEY2 apply seed
        }

        value |= self.key2_gap << 16; // ROMCTRL bit 16: KEY1 gap2 length (bits 16-21)
        if self.key2_cmd_enabled {
            value |= 1 << 22; // ROMCTRL bit 22: KEY2 encrypt command
        }
        if self.word_ready {
            value |= 1 << 23; // ROMCTRL bit 23: data word status (ready)
        }
        value |= self.block_size << 24; // ROMCTRL bit 24: data block size (bits 24-26)
        if self.slow_transfer {
            value |= 1 << 27; // ROMCTRL bit 27: transfer clock rate
        }
        if self.key1_gap_clocks {
            value |= 1 << 28; // ROMCTRL bit 28: KEY1 gap clocks
        }
        value |= 1 << 29; //Always one? Who knows
        if self.write_dir {
            value |= 1 << 30; // ROMCTRL bit 30: data direction write
        }

        if self.block_busy {
            value |= 1 << 31; // ROMCTRL bit 31: block start / busy
        }

        value
    }

    /// Set register value from 32-bit word
    pub const fn set(&mut self, value: u32) {
        // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
        self.key1_gap = value & 0x3F; // KEY1 gap length (bits 0-5)
        // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
        self.key2_data_enabled = (value & (1 << 6)) != 0; // bit 6 of this packed value: KEY2 data enable
        self.key2_apply_seed = (value & (1 << 7)) != 0; // bit 7: KEY2 apply seed
        self.key2_gap = (value >> 8) & 0x3F; // KEY2 gap length (bits 8-13)
        self.key2_cmd_enabled = (value & (1 << 14)) != 0; // bit 14: KEY2 command enable
        self.word_ready = (value & (1 << 15)) != 0; // bit 15: data word ready
        self.block_size = (value >> 16) & 0x7; // data block size (bits 16-18)
        self.slow_transfer = (value & (1 << 19)) != 0; // bit 19: slow transfer clock
        self.key1_gap_clocks = (value & (1 << 20)) != 0; // bit 20: KEY1 gap clocks
        self.block_busy = (value & (1 << 31)) != 0; // bit 31: block busy
    }
}

impl Default for RegRomCtrl {
    fn default() -> Self {
        Self::new()
    }
}

/// Game Cartridge Controller
/// Manages ROM data, encryption/decryption, and cartridge interface
impl NDSCart {
    /// Reads a command byte from command buffer.
    pub fn read_command(&self, index: usize) -> u8 {
        self.command_buffer.get(index).copied().unwrap_or(0)
    }

    /// Writes a command byte into command buffer (ARM7/ARM9 writes to
    /// 0x040001A8..0x040001AF; the command runs on the next ROMCTRL start).
    pub fn receive_command(&mut self, command: u8, index: usize) {
        if let Some(slot) = self.command_buffer.get_mut(index) {
            *slot = command;
        }
    }

    // Moved functions to struct Emulator.
    // pub fn run(&mut self, cycles: i32);

    /// Reads raw byte from ROM without timing effects.
    pub fn direct_read(&self, address: u32) -> u8 {
        self.rom.get(address as usize).copied().unwrap_or_else(|| {
            #[cfg(feature = "log")]
            log::warn!("Direct read out of bounds: {address:08X}");

            0
        })
    }

    /// Reads little-endian halfword from ROM.
    pub fn direct_read_halfword(&self, address: u32) -> u16 {
        let lo = self.direct_read(address) as u16;
        let hi = self.direct_read(address + 1) as u16;
        lo | (hi << 8)
    }

    /// Reads little-endian word from ROM.
    pub fn direct_read_word(&self, address: u32) -> u32 {
        let lo = self.direct_read_halfword(address) as u32;
        let hi = self.direct_read_halfword(address + 2) as u32;
        lo | (hi << 16) // little-endian halfwords to word
    }

    /// Size of the card's ROM chip: the image size rounded up to a power of
    /// two (melonDS `PadToPowerOf2`).
    fn chip_size(&self) -> u32 {
        // DS Cartridge Protocol: https://problemkaputt.de/gbatek.htm#dscartridgeprotocol
        (self.rom.len().max(0x10_0000) as u32).next_power_of_two() // chip size rounded up to a power of two, minimum 1MB
    }

    /// Chip ID returned by command B8 and stored by the firmware at
    /// 0x027FF800/0x027FFC00: Macronix (0xC2) plus a size code, the chip size
    /// in MiB - 1 up to 128 MiB and `0x100 - size / 256 MiB` above that
    /// (melonDS `ParseROM`, GBATEK "DS Cartridge Protocol").
    pub fn chip_id(&self) -> u32 {
        let size = self.chip_size();
        let size_code = if size <= 128 << 20 {
            // chips up to 128MB use the plain size code
            (size >> 20) - 1
        } else {
            0x100 - (size >> 28) // chip ID size byte: 100h - size in 256MB units for large chips
        };
        let nand = if self.backup.is_nand() { 0x0800_0000 } else { 0 }; // bit 27: NAND card (melonDS)
        0xC2 | size_code << 8 | nand // chip ID: manufacturer C2h (Macronix), size code in byte 1
    }

    /// Reads a word for the KEY2 "Get Data" command (B7).
    ///
    /// The address wraps at the chip size, and the first 32 KiB (secure
    /// area) is not readable: it returns 0x8000-0x81FF instead (melonDS
    /// `CartRetail::ReadROM_B7`). Bytes past the end of a trimmed image read
    /// as 0xFF, like erased chip space. Anti-piracy code in later games
    /// reads past the chip size (e.g. 0x1000_1000 on a 256 MiB card) and
    /// terminates unless the data matches the wrapped address.
    pub fn read_rom_b7_word(&self, address: u32) -> u32 {
        let mut address = address & (self.chip_size() - 1);
        // DS Cartridge Protocol: https://problemkaputt.de/gbatek.htm#dscartridgeprotocol
        if address < 0x8000 {
            // B7 reads below 8000h are redirected
            address = 0x8000 + (address & 0x1FF); // into 8000h-81FFh
        }
        let byte = |a: u32| self.rom.get(a as usize).copied().unwrap_or(0xFF); // unmapped ROM reads FFh
        u32::from_le_bytes([byte(address), byte(address + 1), byte(address + 2), byte(address + 3)])
    }

    /// Returns ROM CTRL register value.
    pub const fn get_romctrl(&self) -> u32 {
        self.romctrl.get()
    }

    /// Returns cartridge output register value (0x04100010 read via ds_emu
    /// cart_read_data); consuming a ready word lets cartridge_run() fetch the
    /// next one 8 cycles later.
    pub const fn get_output(&mut self) -> u32 {
        if self.romctrl.word_ready {
            self.romctrl.word_ready = false;
            self.cycles_left = 8;
        }

        self.data_output
    }

    /// Updates ROMCTRL register and may trigger a new cartridge transfer.
    ///
    /// ```text
    /// from: ds_emu arm7_/arm9_write_word(0x040001A4) (ROMCTRL)
    ///   |
    ///   v
    /// set_romctrl(value)
    ///   1. decode gaps, KEY2 flags, block size, speed, start bit
    ///   2. start bit 0 -> 1 with the slot enabled:
    ///        bytes_left = block size (0, 0x200 << n, or 4)
    ///        KEY1 mode: add the gap, decrypt the 8 command bytes
    ///        decode command_buffer[0]: 9F dummy, 00 header, 90/B8 chip ID,
    ///        3C enable KEY1, B7 ROM data (address from bytes 1-4),
    ///        KEY1-mode commands: 1x chip ID, 2x secure area, 4x dummy,
    ///        Ax enable KEY2
    ///   |
    ///   v
    /// to:   ds_emu cartridge_run() produces the words, read through
    ///       0x04100010 (cart_read_data -> get_output)
    /// ```
    ///
    /// # Panics
    pub fn set_romctrl(&mut self, value: u32) {
        // #[cfg(feature = "log")]
        // log::debug!("Setting ROMCTRL: {value:08X}");

        let old_transfer_busy = self.romctrl.block_busy;

        // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
        self.romctrl.key1_gap = value & 0x1FFF; // ROMCTRL KEY1 gap1 length (bits 0-12)
        self.romctrl.key2_data_enabled = ((value >> 13) & 1) != 0;
        self.romctrl.key2_apply_seed = ((value >> 15) & 1) != 0;
        self.romctrl.key2_gap = (value >> 16) & 0x3F; // ROMCTRL KEY1 gap2 length (bits 16-21)
        self.romctrl.key2_cmd_enabled = ((value >> 22) & 1) != 0;
        self.romctrl.block_size = (value >> 24) & 0x7; // ROMCTRL data block size (bits 24-26)
        self.romctrl.slow_transfer = ((value >> 27) & 1) != 0;
        self.romctrl.key1_gap_clocks = ((value >> 28) & 1) != 0;
        self.romctrl.write_dir = ((value >> 30) & 1) != 0;
        self.romctrl.block_busy = ((value >> 31) & 1) != 0;

        // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
        if !old_transfer_busy && self.romctrl.block_busy && self.auxspicnt & (1 << 15) != 0 {
            // AUXSPICNT bit 15: NDS slot enable
            self.romctrl.word_ready = false;

            self.bytes_left = match self.romctrl.block_size {
                0 => 0,
                7 => 4,
                n => 0x100 << n, // block sizes 100h << n (bits 24-26 = 1..6)
            };

            if self.cmd_encrypt_mode != 0 {
                self.cycles_left += self.romctrl.key1_gap as i32;
            }

            if self.cmd_encrypt_mode == 1 {
                self.cycles_left += self.romctrl.key1_gap as i32;

                let mut data = [0_u8; 8];
                for (data, buffer) in data.iter_mut().zip(self.command_buffer) {
                    *data = buffer;
                }

                // To [u32; 2]
                #[expect(clippy::unwrap_used)]
                let mut data = [
                    u32::from_le_bytes(data[0..4].try_into().unwrap()),
                    u32::from_le_bytes(data[4..8].try_into().unwrap()),
                ];
                self.key1_decrypt(&mut data);

                // #[cfg(feature = "log")]
                // log::trace!(
                //     "Data sent {:02X?} decrypted {data:02X?}",
                //     self.command_buffer,
                // );

                // to bytes
                let mut out = [0_u8; 8];
                out[0..4].copy_from_slice(&data[0].to_le_bytes());
                out[4..8].copy_from_slice(&data[1].to_le_bytes());

                for (buffer, data) in self.command_buffer.iter_mut().rev().zip(out) {
                    *buffer = data;
                }
            }

            self.command_id = CartCommand::Empty;
            #[cfg(feature = "log")]
            log::trace!(target: "card", "cmd {:02X?} len {:X}", self.command_buffer, self.bytes_left);

            let cmd = self.command_buffer;
            if self.cmd_encrypt_mode == 2
                && matches!(cmd[0], 0xC0 | 0xC1)
                && let Some(sd) = &mut self.sd
            {
                sd.start(cmd[0], u32::from_be_bytes([cmd[1], cmd[2], cmd[3], cmd[4]]));
                self.command_id = CartCommand::Sd;
                return;
            }
            if self.cmd_encrypt_mode == 2
                && self.backup.nand_command(&self.command_buffer) == NandCommand::Nand
            {
                self.command_id = CartCommand::Nand;
                return;
            }

            match self.command_buffer[0] {
                // DS Cartridge Protocol: https://problemkaputt.de/gbatek.htm#dscartridgeprotocol
                0x9F => self.command_id = CartCommand::Dummy, // 9Fh: dummy (high-z read)
                0x00 => {
                    // 00h: get header
                    self.command_id = CartCommand::GetHeader;
                    self.rom_data_index = 0;
                }
                0x90 | 0xB8 => self.command_id = CartCommand::GetChipId, // 90h / B8h: get chip ID
                0x3C => self.command_id = CartCommand::EnableKey1, // 3Ch: activate KEY1 encryption
                0xB7 => {
                    // B7h: encrypted data read
                    self.command_id = CartCommand::ReadRom;

                    self.rom_data_index = (((self.command_buffer[1] as u32) << 24) // B7 address byte (big-endian, command byte 1)
                        | ((self.command_buffer[2] as u32) << 16) // B7 address byte (big-endian, command byte 2)
                        | ((self.command_buffer[3] as u32) << 8) // B7 address byte (big-endian, command byte 3)
                        | (self.command_buffer[4] as u32))
                        as usize;

                    if self.bytes_left > 0x1000 {
                        // reads are capped at 1000h bytes
                        #[cfg(feature = "log")]
                        log::error!("ROM read bytes_left > 0x1000"); // reads are capped at 1000h bytes
                    }
                }

                // DS Cartridge Secure Area: https://problemkaputt.de/gbatek.htm#dscartridgesecurearea
                _ => match self.command_buffer[0] & 0xF0 {
                    // KEY1 command (bits 4-7 of the decrypted first byte)
                    0x40 => self.command_id = CartCommand::Dummy, // 4xh: activate KEY2
                    0x10 => self.command_id = CartCommand::GetChipId, // 1xh: get chip ID
                    0x20 => {
                        // 2xh: get secure area block
                        self.command_id = CartCommand::GetSecureAreaBlock;
                        self.secure_area_index = ((self.command_buffer[2] & 0xF0) as u32) << 8; // secure area block number (bits 4-7) in 4K steps
                    }
                    0xA0 => {
                        // Axh: enter main data mode
                        self.command_id = CartCommand::Dummy;
                        self.cmd_encrypt_mode = 2;
                    }
                    _ => {}
                },
            }
        }
    }
}
