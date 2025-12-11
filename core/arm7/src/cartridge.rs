/// Game Cartridge controller for Nintendo DS
/// Handles ROM loading, encryption/decryption, and cartridge access

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
        let mut value = 0u32;
        value |= (self.key1_gap & 0x3F) << 0;
        if self.key2_data_enabled {
            value |= 1 << 6;
        }
        if self.key2_apply_seed {
            value |= 1 << 7;
        }
        value |= (self.key2_gap & 0x3F) << 8;
        if self.key2_cmd_enabled {
            value |= 1 << 14;
        }
        if self.word_ready {
            value |= 1 << 15;
        }
        value |= (self.block_size & 0x7) << 16;
        if self.slow_transfer {
            value |= 1 << 19;
        }
        if self.key1_gap_clocks {
            value |= 1 << 20;
        }
        if self.block_busy {
            value |= 1 << 31;
        }
        value
    }

    /// Set register value from 32-bit word
    pub fn set(&mut self, value: u32) {
        self.key1_gap = (value >> 0) & 0x3F;
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
pub struct NDSCart {
    /// Emulator reference
    emulator_ptr: *mut crate::emulator::Emulator,

    /// KEY1 encryption buffer (0x1048 bytes)
    key1_buffer: Vec<u8>,
    /// Encryption mode (0=normal, 1=encrypted)
    cmd_encrypt_mode: u32,

    /// ROM data
    rom: Vec<u8>,
    /// Save database
    save_database: Vec<u8>,
    /// ROM filename
    rom_name: String,
    /// SPI save data (8MB)
    spi_save: Vec<u8>,
    /// Save size in bytes
    save_size: usize,
    /// Save type
    save_type: u32,
    /// Save data has been modified
    dirty_save: bool,
    /// Database size
    database_size: u64,
    /// ROM size
    rom_size: u64,

    /// Command buffer
    command_buffer: [u8; 8],
    /// Data output from cartridge
    data_output: u32,
    /// Current index in ROM data
    rom_data_index: usize,
    /// Current command being executed
    command_id: CartCommand,

    /// Secure area block index
    secure_area_index: u32,

    /// Cycles remaining for current operation
    cycles_left: i32,
    /// Bytes remaining to transfer
    bytes_left: i32,

    /// ROM control register
    romctrl: RegRomCtrl,
    /// Auxiliary SPI control register
    auxspicnt: RegAuxSpiCnt,
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
    keycode: [u32; 3],
}

impl NDSCart {
    /// Create new cartridge controller
    pub fn new() -> Self {
        NDSCart {
            emulator_ptr: std::ptr::null_mut(),
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

    /// Power on cartridge
    pub fn power_on(&mut self) -> Result<(), String> {
        self.command_id = CartCommand::Empty;
        self.cycles_left = 0;
        self.bytes_left = 0;
        Ok(())
    }

    /// Load save game database
    pub fn load_database(&mut self, _file_name: &str) -> Result<usize, String> {
        Ok(0)
    }

    /// Load ROM from file
    pub fn load_rom(&mut self, _file_name: &str) -> Result<usize, String> {
        Ok(0)
    }

    /// Check and save dirty save data
    pub fn save_check(&mut self) -> Result<(), String> {
        if self.dirty_save {
            // Write save data to file
            self.dirty_save = false;
        }
        Ok(())
    }

    /// Read command byte
    pub fn read_command(&self, index: usize) -> u8 {
        if index < 8 {
            self.command_buffer[index]
        } else {
            0
        }
    }

    /// Receive command byte
    pub fn receive_command(&mut self, command: u8, index: usize) {
        if index < 8 {
            self.command_buffer[index] = command;
        }
    }

    /// Run cartridge for specified cycles
    pub fn run(&mut self, _cycles: i32) -> Result<(), String> {
        Ok(())
    }

    /// Debug encryption (for testing)
    pub fn debug_encrypt(&mut self) {
        // Encryption testing function
    }

    /// Direct read from ROM
    pub fn direct_read(&self, address: u32) -> u8 {
        let idx = address as usize;
        if idx < self.rom.len() {
            self.rom[idx]
        } else {
            0
        }
    }

    /// Direct read halfword from ROM
    pub fn direct_read_halfword(&self, address: u32) -> u16 {
        let lo = self.direct_read(address) as u16;
        let hi = self.direct_read(address + 1) as u16;
        lo | (hi << 8)
    }

    /// Direct read word from ROM
    pub fn direct_read_word(&self, address: u32) -> u32 {
        let lo = self.direct_read_halfword(address) as u32;
        let hi = self.direct_read_halfword(address + 2) as u32;
        lo | (hi << 16)
    }

    /// Get ROMCTRL register
    pub fn get_romctrl(&self) -> u32 {
        self.romctrl.get()
    }

    /// Get output register
    pub fn get_output(&self) -> u32 {
        self.data_output
    }

    /// Get AUXSPICNT register
    pub fn get_auxspicnt(&self) -> u16 {
        self.auxspicnt.get()
    }

    /// Read AUXSPIDATA register
    pub fn read_auxspidata(&self) -> u8 {
        self.spi_data
    }

    /// Set high byte of AUXSPICNT
    pub fn set_hi_auxspicnt(&mut self, value: u8) {
        let current = self.auxspicnt.get();
        let new_val = (current & 0xFF) | ((value as u16) << 8);
        self.auxspicnt.set(new_val);
    }

    /// Set AUXSPICNT register
    pub fn set_auxspicnt(&mut self, value: u16) {
        self.auxspicnt.set(value);
    }

    /// Write AUXSPIDATA register
    pub fn set_auxspidata(&mut self, value: u8) {
        self.spi_data = value;
    }

    /// Set ROMCTRL register
    pub fn set_romctrl(&mut self, value: u32) {
        self.romctrl.set(value);
    }

    /// Set low KEY2 seed 0
    pub fn set_lo_key2_seed0(&mut self, word: u32) {
        self.encrypt_seed0 = (self.encrypt_seed0 & 0xFFFFFFFF00000000) | (word as u64);
    }

    /// Set low KEY2 seed 1
    pub fn set_lo_key2_seed1(&mut self, word: u32) {
        self.encrypt_seed1 = (self.encrypt_seed1 & 0xFFFFFFFF00000000) | (word as u64);
    }

    /// Set high KEY2 seed 0
    pub fn set_hi_key2_seed0(&mut self, word: u32) {
        self.encrypt_seed0 = (self.encrypt_seed0 & 0x00000000FFFFFFFF) | ((word as u64) << 32);
    }

    /// Set high KEY2 seed 1
    pub fn set_hi_key2_seed1(&mut self, word: u32) {
        self.encrypt_seed1 = (self.encrypt_seed1 & 0x00000000FFFFFFFF) | ((word as u64) << 32);
    }

    // Private encryption methods

    /// KEY1 encrypt data
    fn key1_encrypt(&mut self, _data: &mut [u32]) {
        // KEY1 encryption algorithm
    }

    /// KEY1 decrypt data
    fn key1_decrypt(&mut self, _data: &mut [u32]) {
        // KEY1 decryption algorithm
    }

    /// Apply keycode for encryption
    fn apply_keycode(&mut self, _modulo: u32) {
        // Apply encryption keycode
    }

    /// Initialize keycode from cartridge ID
    fn init_keycode(&mut self, _idcode: u32, _level: i32, _modulo: u32) {
        // Initialize encryption keycode from chip ID and level
    }
}

impl Default for NDSCart {
    fn default() -> Self {
        Self::new()
    }
}
