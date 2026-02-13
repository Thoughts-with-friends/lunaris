//! Game Cartridge controller for Nintendo DS
//! Handles ROM loading, encryption/decryption, and cartridge access

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Cartridge command types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl CartCommand {
    /// Convert numeric value to CartCommand
    pub fn from_value(val: u32) -> Self {
        match val {
            0 => CartCommand::Empty,
            1 => CartCommand::Dummy,
            2 => CartCommand::GetHeader,
            3 => CartCommand::GetChipId,
            4 => CartCommand::EnableKey1,
            5 => CartCommand::EnableKey2,
            6 => CartCommand::GetSecureAreaBlock,
            7 => CartCommand::ReadRom,
            _ => CartCommand::Empty,
        }
    }
}

/// Auxiliary SPI command types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxSpiCommand {
    /// No command
    Empty = 0,
    /// Write to memory
    WriteMem = 1,
    /// Read from memory
    ReadMem = 2,
    /// Read status register
    ReadStatusReg = 3,
    /// Page write operation
    PageWrite = 4,
    /// Write high address
    WriteHi = 5,
    /// Read high address
    ReadHi = 6,
}

impl AuxSpiCommand {
    /// Convert numeric value to AuxSpiCommand
    pub fn from_value(val: u32) -> Self {
        match val {
            0 => AuxSpiCommand::Empty,
            1 => AuxSpiCommand::WriteMem,
            2 => AuxSpiCommand::ReadMem,
            3 => AuxSpiCommand::ReadStatusReg,
            4 => AuxSpiCommand::PageWrite,
            5 => AuxSpiCommand::WriteHi,
            6 => AuxSpiCommand::ReadHi,
            _ => AuxSpiCommand::Empty,
        }
    }
}

/// ROM control register
#[derive(Debug, Clone, Copy)]
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
}

impl RegRomCtrl {
    /// Create new ROM control register
    pub fn new() -> Self {
        RegRomCtrl {
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
        }
    }

    /// Get register value as 32-bit word
    pub fn get(&self) -> u32 {
        let mut value = 0;
        value |= self.key1_gap;
        if self.key2_data_enabled {
            value |= 1 << 13;
        }
        if self.key2_apply_seed {
            value |= 1 << 15;
        }

        value |= self.key2_gap << 16;
        if self.key2_cmd_enabled {
            value |= 1 << 22;
        }
        if self.word_ready {
            value |= 1 << 23;
        }
        value |= self.block_size << 24;
        if self.slow_transfer {
            value |= 1 << 27;
        }
        if self.key1_gap_clocks {
            value |= 1 << 28;
        }
        value |= 1 << 29; //Always one? Who knows

        if self.block_busy {
            value |= 1 << 31;
        }

        value
    }

    /// Set register value from 32-bit word
    pub fn set(&mut self, value: u32) {
        self.key1_gap = value & 0x3F;
        self.key2_data_enabled = (value & (1 << 6)) != 0;
        self.key2_apply_seed = (value & (1 << 7)) != 0;
        self.key2_gap = (value >> 8) & 0x3F;
        self.key2_cmd_enabled = (value & (1 << 14)) != 0;
        self.word_ready = (value & (1 << 15)) != 0;
        self.block_size = (value >> 16) & 0x7;
        self.slow_transfer = (value & (1 << 19)) != 0;
        self.key1_gap_clocks = (value & (1 << 20)) != 0;
        self.block_busy = (value & (1 << 31)) != 0;
    }
}

impl Default for RegRomCtrl {
    fn default() -> Self {
        Self::new()
    }
}

/// Auxiliary SPI control register
#[derive(Debug, Clone, Copy)]
pub struct RegAuxSpiCnt {
    /// Baud rate divider
    pub bandwidth: u32,
    /// Hold chip select after transfer
    pub hold_chipselect: bool,
    /// Transfer in progress
    pub is_busy: bool,
    /// Serial transfer mode
    pub serial_transfer: bool,
    /// Generate interrupt after transfer
    pub irq_after_transfer: bool,
    /// SPI enabled
    pub enabled: bool,
}

impl RegAuxSpiCnt {
    /// Create new auxiliary SPI control register
    pub fn new() -> Self {
        RegAuxSpiCnt {
            bandwidth: 0,
            hold_chipselect: false,
            is_busy: false,
            serial_transfer: false,
            irq_after_transfer: false,
            enabled: false,
        }
    }

    /// Get register value as 16-bit halfword
    pub fn get(&self) -> u16 {
        let mut value = 0u16;
        value |= (self.bandwidth & 0x3) as u16;
        if self.hold_chipselect {
            value |= 1 << 3;
        }
        if self.is_busy {
            value |= 1 << 7;
        }
        if self.serial_transfer {
            value |= 1 << 13;
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
        self.hold_chipselect = (value & (1 << 3)) != 0;
        self.serial_transfer = (value & (1 << 13)) != 0;
        self.irq_after_transfer = (value & (1 << 14)) != 0;
        self.enabled = (value & (1 << 15)) != 0;
    }
}

impl Default for RegAuxSpiCnt {
    fn default() -> Self {
        Self::new()
    }
}

/// Game Cartridge Controller
/// Manages ROM data, encryption/decryption, and cartridge interface
#[derive(Debug)]
pub struct NDSCart {
    /// KEY1 encryption buffer (0x1048 bytes)
    pub key1_buffer: Vec<u8>,
    /// Encryption mode (0=normal, 1=encrypted)
    pub(crate) cmd_encrypt_mode: u32,

    /// ROM data
    pub(crate) rom: Vec<u8>,
    /// Save database
    pub(crate) save_database: Vec<u8>,
    /// ROM filename
    pub(crate) rom_name: String,
    /// SPI save data (8MB)
    pub(crate) spi_save: Vec<u8>,
    /// Save size in bytes
    pub(crate) save_size: usize,
    /// Save type
    pub(crate) save_type: u32,
    /// Save data has been modified
    pub(crate) dirty_save: bool,
    /// Database size
    pub(crate) database_size: u64,
    /// ROM size
    pub(crate) rom_size: u64,

    /// Command buffer
    pub(crate) command_buffer: [u8; 8],
    /// Data output from cartridge
    pub(crate) data_output: u32,
    /// Current index in ROM data
    pub(crate) rom_data_index: usize,
    /// Current command being executed
    pub(crate) command_id: CartCommand,

    /// Secure area block index
    pub(crate) secure_area_index: u32,

    /// Cycles remaining for current operation
    pub(crate) cycles_left: i32,
    /// Bytes remaining to transfer
    pub(crate) bytes_left: i32,

    /// ROM control register
    pub(crate) romctrl: RegRomCtrl,
    /// Auxiliary SPI control register
    pub(crate) auxspicnt: RegAuxSpiCnt,
    /// Current SPI command
    spi_cmd: AuxSpiCommand,
    /// SPI data byte
    spi_data: u8,
    /// SPI command parameters
    spi_params: i32,
    /// SPI address for read/write
    spi_addr: u32,
    /// SPI write enabled flag
    spi_write_enabled: bool,

    /// Encryption seed 0
    encrypt_seed0: u64,
    /// Encryption seed 1
    encrypt_seed1: u64,

    /// Keycode for encryption (3 words)
    pub(crate) keycode: [u32; 3],
}

impl Default for NDSCart {
    fn default() -> Self {
        Self::new()
    }
}

impl NDSCart {
    /// Create new cartridge controller
    pub fn new() -> Self {
        NDSCart {
            key1_buffer: vec![0u8; 0x1048],
            cmd_encrypt_mode: 0,
            rom: Vec::new(),
            save_database: Vec::new(),
            rom_name: String::new(),
            spi_save: vec![0u8; 1024 * 1024 * 8],
            save_size: 0,
            save_type: 0,
            dirty_save: false,
            database_size: 0,
            rom_size: 0,
            command_buffer: [0u8; 8],
            data_output: 0,
            rom_data_index: 0,
            command_id: CartCommand::Empty,
            secure_area_index: 0,
            cycles_left: 0,
            bytes_left: 0,
            romctrl: RegRomCtrl::new(),
            auxspicnt: RegAuxSpiCnt::new(),
            spi_cmd: AuxSpiCommand::Empty,
            spi_data: 0,
            spi_params: 0,
            spi_addr: 0,
            spi_write_enabled: false,
            encrypt_seed0: 0,
            encrypt_seed1: 0,
            keycode: [0u32; 3],
        }
    }

    /// Resets cartridge state to power-on defaults.
    pub fn power_on(&mut self) {
        self.save_size = 1024 * 1024;
        self.save_type = 2;
        self.cycles_left = 8;
        self.bytes_left = 0;
        self.romctrl.word_ready = true;
        self.romctrl.block_busy = false;
        self.cmd_encrypt_mode = 0;
        self.auxspicnt.hold_chipselect = false;
        self.auxspicnt.is_busy = false;
        self.auxspicnt.serial_transfer = false;
        self.auxspicnt.irq_after_transfer = false;
        self.auxspicnt.enabled = false;

        self.spi_save.clear();
        self.dirty_save = false;

        if !self.rom.is_empty() {
            self.rom.clear();
        }

        self.rom_name.clear();

        self.spi_cmd = AuxSpiCommand::Empty;
        self.spi_data = 0;
        self.spi_params = 0;
    }

    /// Loads save database file into memory.
    pub fn load_database(&mut self, file_name: &Path) -> Result<(), CartridgeError> {
        use std::fs::File;
        use std::io::Read;

        self.save_database.clear();
        self.database_size = 0;

        #[cfg(feature = "tracing")]
        tracing::info!("Loading save database: {}", file_name.display());

        let mut file = File::open(&file_name).map_err(|source| {
            #[cfg(feature = "tracing")]
            tracing::error!("Failed to open save database: {}", file_name.display());

            CartridgeError::OpenSaveDatabase {
                path: file_name.to_path_buf(),
                source,
            }
        })?;

        let metadata = std::fs::metadata(&file_name).map_err(|source| {
            #[cfg(feature = "tracing")]
            tracing::error!("Failed to read metadata: {}", file_name.display());

            CartridgeError::MetadataSaveDatabase {
                path: file_name.to_path_buf(),
                source,
            }
        })?;

        let size = metadata.len();
        self.database_size = size;

        if size % 19 != 0 {
            #[cfg(feature = "tracing")]
            tracing::error!(
                "Invalid save database format: {} size={size}",
                file_name.display(),
            );

            return Err(CartridgeError::InvalidSaveDatabaseFormat {
                path: file_name.to_path_buf(),
                size,
            });
        }

        self.save_database.resize(size as usize, 0);

        file.read_exact(&mut self.save_database).map_err(|source| {
            #[cfg(feature = "tracing")]
            tracing::error!(
                "Failed to read save database contents: {}",
                file_name.display()
            );

            CartridgeError::ReadSaveDatabase {
                path: file_name.to_path_buf(),
                source,
            }
        })?;

        #[cfg(feature = "tracing")]
        tracing::info!("Save database successfully loaded: {}", file_name.display());

        Ok(())
    }

    // Move to struct Emulator
    /// Loads ROM image and optional save data.
    // pub fn load_rom( &mut self, file_name: &Path, is_direct_boot_enabled: bool,) -> Result<(), CartridgeError>;

    /// Writes save data to disk if modified.
    pub fn save_check(&mut self) -> Result<(), CartridgeError> {
        if !self.dirty_save {
            return Ok(());
        }
        self.dirty_save = false;

        let path = PathBuf::from(format!("{}.sav", self.rom_name));

        #[cfg(feature = "tracing")]
        tracing::debug!("Flushing save file: {}", path.display());

        let mut file = std::fs::File::create(&path).map_err(|source| CartridgeError::SaveOpen {
            path: path.clone(),
            source,
        })?;

        file.write_all(&self.spi_save)
            .map_err(|source| CartridgeError::SaveWrite { path, source })?;

        Ok(())
    }

    /// Reads a command byte from command buffer.
    pub fn read_command(&self, index: usize) -> u8 {
        self.command_buffer.get(index).copied().unwrap_or(0)
    }

    /// Writes a command byte into command buffer.
    pub fn receive_command(&mut self, command: u8, index: usize) {
        if let Some(slot) = self.command_buffer.get_mut(index) {
            *slot = command;
        }
    }

    // Moved functions to struct Emulator.
    // pub fn run(&mut self, cycles: i32);

    /// Runs debug encryption test routine.
    pub fn debug_encrypt(&mut self) {
        todo!()
    }

    /// Reads raw byte from ROM without timing effects.
    pub fn direct_read(&self, address: u32) -> u8 {
        self.rom.get(address as usize).copied().unwrap_or(0)
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
        lo | (hi << 16)
    }

    /// Returns ROM CTRL register value.
    pub fn get_romctrl(&self) -> u32 {
        self.romctrl.get()
    }

    /// Returns cartridge output register value.
    pub fn get_output(&mut self) -> u32 {
        if (self.romctrl.word_ready) {
            self.romctrl.word_ready = false;
            self.cycles_left = 8;
        }

        self.data_output
    }

    /// Returns AUXSPICNT register value.
    pub fn get_auxspicnt(&self) -> u16 {
        todo!()
    }

    /// Reads AUXSPIDATA register value.
    pub fn read_auxspidata(&self) -> u8 {
        self.spi_data
    }

    /// Sets high byte of AUXSPICNT register.
    pub fn set_hi_auxspicnt(&mut self, value: u8) {
        self.auxspicnt.serial_transfer = (value & (1 << 5)) != 0;
        self.auxspicnt.irq_after_transfer = (value & (1 << 6)) != 0;
        self.auxspicnt.enabled = (value & (1 << 7)) != 0;
    }

    /// Sets AUXSPICNT register value.
    pub fn set_auxspicnt(&mut self, value: u16) {
        self.auxspicnt.bandwidth = (value & 0x3) as u32;
        self.auxspicnt.hold_chipselect = (value & (1 << 6)) != 0;
        self.set_hi_auxspicnt((value >> 8) as u8);
    }

    /// Handles AUX SPI data write and updates SPI state machine.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    pub fn set_auxspidata(&mut self, value: u8) {
        if self.spi_cmd == AuxSpiCommand::Empty {
            self.spi_params = 0;

            match value {
                0 => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!("AUXSPI derp");
                }

                2 => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!("AUXSPI write");
                    self.spi_cmd = AuxSpiCommand::WriteMem;
                }

                3 => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!("AUXSPI read");
                    self.spi_cmd = AuxSpiCommand::ReadMem;
                }

                4 => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!("AUXSPI write disabled");
                    self.spi_write_enabled = false;
                }

                5 => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!("AUXSPI read status reg");
                    self.spi_cmd = AuxSpiCommand::ReadStatusReg;
                    self.spi_addr = 0;
                }

                6 => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!("AUXSPI write enabled");
                    self.spi_write_enabled = true;
                }

                10 => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!("AUXSPI page write");

                    self.spi_cmd = if self.save_type == 2 {
                        AuxSpiCommand::PageWrite
                    } else {
                        AuxSpiCommand::WriteHi
                    };
                }

                11 => {
                    if self.save_type == 0 {
                        self.spi_cmd = AuxSpiCommand::ReadHi;
                    }
                }

                _ => {
                    #[cfg(feature = "tracing")]
                    tracing::error!("Unrecognized AUXSPI cmd {}", value);

                    #[cfg(not(feature = "tracing"))]
                    eprintln!("Unrecognized AUXSPI cmd {}", value);

                    return;
                }
            }
        } else {
            match self.spi_cmd {
                AuxSpiCommand::ReadStatusReg => {
                    self.spi_data = (self.spi_write_enabled as u8) << 1;
                    if self.save_type == 0 {
                        self.spi_data |= 0xF0;
                    }
                }

                AuxSpiCommand::WriteMem => {
                    match self.save_type {
                        0 => {
                            if self.spi_params < 2 {
                                self.spi_addr = value as u32;
                            } else if self.spi_write_enabled {
                                self.dirty_save = true;
                                let idx = (self.spi_addr & 0xFF) as usize;
                                self.spi_save[idx] = value;
                                self.spi_addr += 1;
                            }
                        }

                        1 => {
                            if self.spi_params < 3 {
                                self.spi_addr |= (value as u32) << ((2 - self.spi_params) * 8);
                            } else if self.spi_write_enabled {
                                self.dirty_save = true;
                                let idx = (self.spi_addr & (self.save_size as u32 - 1)) as usize;
                                self.spi_save[idx] = value;
                                self.spi_addr += 1;
                            }
                        }

                        2 => {
                            if self.spi_params < 4 {
                                self.spi_addr |= (value as u32) << ((3 - self.spi_params) * 8);
                            } else if self.spi_write_enabled {
                                self.dirty_save = true;
                                let idx = (self.spi_addr & (self.save_size as u32 - 1)) as usize;
                                self.spi_save[idx] = 0;
                                self.spi_addr += 1;
                            }
                        }

                        _ => {}
                    }

                    #[cfg(feature = "tracing")]
                    tracing::trace!("WRITE_MEM {}", value);
                }

                AuxSpiCommand::ReadMem => {
                    match self.save_type {
                        0 => {
                            if self.spi_params < 2 {
                                self.spi_addr |= value as u32;
                            } else {
                                let idx = (self.spi_addr & 0xFF) as usize;
                                self.spi_data = self.spi_save[idx];
                                self.spi_addr += 1;
                            }
                        }

                        1 => {
                            if self.spi_params < 3 {
                                self.spi_addr |= (value as u32) << ((2 - self.spi_params) * 8);
                            } else {
                                let idx = (self.spi_addr & (self.save_size as u32 - 1)) as usize;
                                self.spi_data = self.spi_save[idx];
                                self.spi_addr += 1;
                            }
                        }

                        2 => {
                            if self.spi_params < 4 {
                                self.spi_addr <<= 8;
                                self.spi_addr |= value as u32;
                            } else {
                                let idx = (self.spi_addr & (self.save_size as u32 - 1)) as usize;
                                self.spi_data = self.spi_save[idx];
                                self.spi_addr += 1;
                            }
                        }

                        _ => {}
                    }

                    #[cfg(feature = "tracing")]
                    tracing::trace!("READ_MEM {}", value);
                }

                AuxSpiCommand::PageWrite => {
                    if self.save_type == 2 {
                        if self.spi_params < 4 {
                            self.spi_addr <<= 8;
                            self.spi_addr |= value as u32;
                        } else if self.spi_write_enabled {
                            #[cfg(feature = "tracing")]
                            tracing::trace!("Page write {:08X}", self.spi_addr);

                            self.dirty_save = true;
                            let idx = (self.spi_addr & (self.save_size as u32 - 1)) as usize;
                            self.spi_save[idx] = value;
                            self.spi_addr += 1;
                        }
                    }
                }

                AuxSpiCommand::WriteHi => {
                    if self.spi_params < 2 {
                        self.spi_addr = 0x100 | value as u32;
                    } else if self.spi_write_enabled {
                        self.dirty_save = true;
                        let idx = (self.spi_addr & 0x1FF) as usize;
                        self.spi_save[idx] = value;
                        self.spi_addr += 1;

                        if self.spi_addr == 0x200 {
                            self.spi_addr = 0x100;
                        }
                    }
                }

                AuxSpiCommand::ReadHi => {
                    if self.spi_params < 2 {
                        self.spi_addr = 0x100 | value as u32;
                    } else {
                        let idx = (self.spi_addr & 0x1FF) as usize;
                        self.spi_data = self.spi_save[idx];
                        self.spi_addr += 1;

                        if self.spi_addr == 0x200 {
                            self.spi_addr = 0x100;
                        }
                    }
                }

                AuxSpiCommand::Empty => {}
            }
        }

        self.spi_params += 1;

        if !self.auxspicnt.hold_chipselect {
            #[cfg(feature = "tracing")]
            tracing::trace!("Deselected AUXSPI");

            self.spi_params = 0;
            self.spi_addr = 0;
            self.spi_cmd = AuxSpiCommand::Empty;
        }
    }

    /// Updates ROMCTRL register and may trigger a new cartridge transfer.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    pub fn set_romctrl(&mut self, value: u32) {
        let old_transfer_busy = self.romctrl.block_busy;

        self.romctrl.key1_gap = value & 0x1FFF;
        self.romctrl.key2_data_enabled = ((value >> 13) & 1) != 0;
        self.romctrl.key2_apply_seed = ((value >> 15) & 1) != 0;
        self.romctrl.key2_gap = (value >> 16) & 0x3F;
        self.romctrl.key2_cmd_enabled = ((value >> 22) & 1) != 0;
        self.romctrl.block_size = (value >> 24) & 0x7;
        self.romctrl.slow_transfer = ((value >> 27) & 1) != 0;
        self.romctrl.key1_gap_clocks = ((value >> 28) & 1) != 0;
        self.romctrl.block_busy = ((value >> 31) & 1) != 0;

        if !old_transfer_busy && self.romctrl.block_busy && self.auxspicnt.enabled {
            self.romctrl.word_ready = false;

            self.bytes_left = match self.romctrl.block_size {
                0 => 0,
                7 => 4,
                n => 0x100 << n,
            };

            if self.cmd_encrypt_mode != 0 {
                self.cycles_left += self.romctrl.key1_gap as i32;
            }

            if self.cmd_encrypt_mode == 1 {
                self.cycles_left += self.romctrl.key1_gap as i32;

                let mut data = [0u8; 8];

                for (data, buffer) in data.iter_mut().zip(self.command_buffer) {
                    *data = buffer;
                }

                #[cfg(feature = "tracing")]
                tracing::trace!(
                    "Data sent {:02X?} decrypted {:02X?}",
                    self.command_buffer,
                    data
                );

                for i in 0..8 {
                    self.command_buffer[7 - i] = data[i];
                }
            }

            self.command_id = CartCommand::Empty;

            match self.command_buffer[0] {
                0x9F => self.command_id = CartCommand::Dummy,

                0x00 => {
                    self.command_id = CartCommand::GetHeader;
                    self.rom_data_index = 0;
                }

                0x90 => self.command_id = CartCommand::GetChipId,

                0x3C => self.command_id = CartCommand::EnableKey1,

                0xB7 => {
                    self.command_id = CartCommand::ReadRom;

                    self.rom_data_index = (((self.command_buffer[1] as u32) << 24)
                        | ((self.command_buffer[2] as u32) << 16)
                        | ((self.command_buffer[3] as u32) << 8)
                        | (self.command_buffer[4] as u32))
                        as usize;

                    if self.bytes_left > 0x1000 {
                        #[cfg(feature = "tracing")]
                        tracing::error!("ROM read bytes_left > 0x1000");

                        #[cfg(not(feature = "tracing"))]
                        eprintln!("ROM read bytes_left > 0x1000");

                        return;
                    }
                }

                0xB8 => self.command_id = CartCommand::GetChipId,

                _ => match self.command_buffer[0] & 0xF0 {
                    0x40 => {
                        self.command_id = CartCommand::Dummy;
                    }

                    0x10 => {
                        self.command_id = CartCommand::GetChipId;
                    }

                    0x20 => {
                        self.command_id = CartCommand::GetSecureAreaBlock;

                        self.secure_area_index = ((self.command_buffer[2] & 0xF0) as u32) << 8;
                    }

                    0xA0 => {
                        self.command_id = CartCommand::Dummy;
                        self.cmd_encrypt_mode = 2;
                    }

                    _ => {}
                },
            }
        }
    }

    /// Sets low 32 bits of KEY2 seed 0.
    pub fn set_lo_key2_seed0(&mut self, word: u32) {
        self.encrypt_seed0 >>= 32;
        self.encrypt_seed0 <<= 32;
        self.encrypt_seed0 |= word as u64;
    }

    /// Sets low 32 bits of KEY2 seed 1.
    pub fn set_lo_key2_seed1(&mut self, word: u32) {
        self.encrypt_seed1 >>= 32;
        self.encrypt_seed1 <<= 32;
        self.encrypt_seed1 |= word as u64;
    }

    /// Sets high 32 bits of KEY2 seed 0.
    pub fn set_hi_key2_seed0(&mut self, word: u32) {
        self.encrypt_seed0 = (self.encrypt_seed0 & 0x00000000FFFFFFFF) | ((word as u64) << 32);
        let word = word & 0x7F;
        self.encrypt_seed0 <<= 32;
        self.encrypt_seed0 >>= 32;
        self.encrypt_seed0 |= (word << 32) as u64;
    }

    /// Sets high 32 bits of KEY2 seed 1.
    pub fn set_hi_key2_seed1(&mut self, word: u32) {
        self.encrypt_seed1 = (self.encrypt_seed1 & 0x00000000FFFFFFFF) | ((word as u64) << 32);
    }

    #[inline]
    fn read_keybuf_u32(&self, index: usize) -> u32 {
        let i = index * 4;
        u32::from_le_bytes([
            self.key1_buffer[i],
            self.key1_buffer[i + 1],
            self.key1_buffer[i + 2],
            self.key1_buffer[i + 3],
        ])
    }

    #[inline]
    fn write_keybuf_u32(&mut self, index: usize, value: u32) {
        let i = index * 4;
        let b = value.to_le_bytes();
        self.key1_buffer[i] = b[0];
        self.key1_buffer[i + 1] = b[1];
        self.key1_buffer[i + 2] = b[2];
        self.key1_buffer[i + 3] = b[3];
    }

    /// Performs KEY1 encryption on data buffer.
    #[must_use]
    pub(crate) fn key1_encrypt(&mut self, mut y: u32, mut x: u32) -> [u32; 2] {
        for i in 0..=0xF {
            let z = (self.read_keybuf_u32(i) ^ x) as usize;
            x = self.read_keybuf_u32(0x012 + ((z >> 24) & 0xFF));
            x += self.read_keybuf_u32(0x112 + ((z >> 16) & 0xFF));
            x ^= self.read_keybuf_u32(0x212 + ((z >> 8) & 0xFF));
            x += self.read_keybuf_u32(0x312 + (z & 0xFF));
            x ^= y;
            y = z as u32;
        }

        [
            x ^ self.read_keybuf_u32(0x10),
            y ^ self.read_keybuf_u32(0x11),
        ]
    }

    /// Performs KEY1 decryption on data buffer.
    fn key1_decrypt(&mut self, data: &mut [u32]) {
        let mut y = data[0];
        let mut x = data[1];

        for i in (0x2..=0x11).rev() {
            let z = self.read_keybuf_u32(i) ^ x;

            x = self.read_keybuf_u32(0x012 + ((z >> 24) as usize & 0xFF));
            x = x.wrapping_add(self.read_keybuf_u32(0x112 + ((z >> 16) as usize & 0xFF)));
            x ^= self.read_keybuf_u32(0x212 + ((z >> 8) as usize & 0xFF));
            x = x.wrapping_add(self.read_keybuf_u32(0x312 + (z as usize & 0xFF)));
            x ^= y;

            y = z;
        }

        data[0] = x ^ self.read_keybuf_u32(1);
        data[1] = y ^ self.read_keybuf_u32(0);
    }

    /// Applies keycode transformation.
    pub(crate) fn apply_keycode(&mut self, modulo: u32) {
        {
            let y = self.keycode[1];
            let x = self.keycode[2];
            let data = self.key1_encrypt(y, x);
            self.keycode[1] = data[0];
            self.keycode[2] = data[1];
        }
        {
            let y = self.keycode[0];
            let x = self.keycode[1];
            let data = self.key1_encrypt(y, x);
            self.keycode[0] = data[0];
            self.keycode[1] = data[1];
        }

        for i in 0..0x11 {
            let mut value = self.read_keybuf_u32(i);
            value ^= byteswap_word(self.keycode[i % modulo as usize]);
            self.write_keybuf_u32(i, value);
        }

        let mut scratch = [0; 2];
        for i in (0..=0x410).skip(1) {
            {
                let y = scratch[0];
                let x = scratch[1];
                let data = self.key1_encrypt(y, x);
                scratch[0] = data[0];
                scratch[1] = data[1];
            }

            self.write_keybuf_u32(i, scratch[1]);
            self.write_keybuf_u32(i + 1, scratch[0]);
        }
    }

    // Movd to struct Emulator method
    // fn init_keycode(&mut self, _idcode: u32, _level: i32, _modulo: u32) ;
}

fn byteswap_word(word: u32) -> u32 {
    let mut result = 0;
    result |= word >> 24;
    result |= (word & 0xFF0000) >> 8;
    result |= (word & 0xFF00) << 8;
    result |= word << 24;
    result
}

/// Cartridge related errors.
#[derive(Debug, snafu::Snafu)]
pub enum CartridgeError {
    /// Save database file could not be opened.
    #[snafu(display("Failed to load save database: {}", path.display()))]
    OpenSaveDatabase {
        path: PathBuf,
        source: std::io::Error,
    },

    /// Save database metadata could not be read.
    #[snafu(display("Failed to read save database metadata: {}", path.display()))]
    MetadataSaveDatabase {
        path: PathBuf,
        source: std::io::Error,
    },

    /// Save database has invalid format.
    #[snafu(display("Save database corrupted or wrong format: {} (size={size})", path.display()))]
    InvalidSaveDatabaseFormat { path: PathBuf, size: u64 },

    /// Save database read failed.
    #[snafu(display("Failed to read save database contents: {}", path.display()))]
    ReadSaveDatabase {
        path: PathBuf,
        source: std::io::Error,
    },

    /// ROM file open failed.
    #[snafu(display("Failed to load ROM: {}", path.display()))]
    OpenRom {
        path: PathBuf,
        source: std::io::Error,
    },

    /// ROM metadata read failed.
    #[snafu(display("Failed to read ROM metadata: {}", path.display()))]
    MetadataRom {
        path: PathBuf,
        source: std::io::Error,
    },

    /// ROM read failed.
    #[snafu(display("Failed to read ROM contents: {}", path.display()))]
    ReadRom {
        path: PathBuf,
        source: std::io::Error,
    },

    /// Save read failed.
    #[snafu(display("Failed to read save file: {}", path.display()))]
    ReadSave {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("Failed to open save file: {}", path.display()))]
    SaveOpen {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("Failed to write save file: {}", path.display()))]
    SaveWrite {
        path: PathBuf,
        source: std::io::Error,
    },
}
