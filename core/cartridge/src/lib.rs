//! Game card slot of the Nintendo DS.
//!
//! ```text
//!            ROM bus                          AUXSPI bus
//!  ROMCMD/ROMCTRL/data (protocol.rs)     AUXSPICNT/AUXSPIDATA (auxspi.rs)
//!       |  KEY1 (key1.rs) / KEY2 (key2.rs)          |
//!       v                                           v
//!   card ROM (.nds image)              backup memory: EEPROM / FRAM / FLASH
//!                                      (backup/, type from game_db.rs)
//! ```
//!
//! [`NDSCart`] holds the loaded ROM, the backup memory and the slot
//! registers; the timing of transfers is driven by the system crate.
use std::path::Path;

mod auxspi;
pub mod backup;
mod dldi_driver;
mod error;
mod fingerprint;
mod foreign;
pub mod game_db;
mod key1;
mod key2;
mod protocol;
pub mod sd;

pub use backup::{BackupMemory, NandCommand, SaveChip, SaveMemType};
pub use error::CartridgeError;
pub use fingerprint::RomFingerprint;
pub use foreign::normalize_foreign_save;
pub use protocol::{CartCommand, RegRomCtrl};

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct NDSCart {
    /// KEY1 encryption buffer (0x1048 bytes)
    pub key1_buffer: Vec<u8>,
    /// Encryption mode (0=normal, 1=encrypted)
    pub cmd_encrypt_mode: u32,

    /// ROM data (host data: not in savestates)
    #[serde(skip)]
    pub rom: Vec<u8>,
    /// Save database
    #[serde(skip)]
    pub save_database: Vec<u8>,
    /// ROM filename
    pub rom_name: String,
    /// Backup memory (save chip) behind AUXSPI
    pub backup: BackupMemory,
    /// Database size
    pub database_size: u64,
    /// ROM size
    pub rom_size: u64,

    /// Command buffer
    pub command_buffer: [u8; 8],
    /// Data output from cartridge
    pub data_output: u32,
    /// Current index in ROM data
    pub rom_data_index: usize,
    /// Current command being executed
    pub command_id: CartCommand,

    /// Secure area block index
    pub secure_area_index: u32,

    /// Cycles remaining for current operation
    pub cycles_left: i32,
    /// Bytes remaining to transfer
    pub bytes_left: i32,

    /// ROM control register
    pub romctrl: RegRomCtrl,
    /// AUXSPICNT (0x040001A0): bits 0-1 baud rate, 6 chip select hold,
    /// 7 busy, 13 SPI mode, 14 transfer IRQ, 15 slot enable
    pub auxspicnt: u16,
    /// Last byte received on AUXSPIDATA (0x040001A2)
    spi_data: u8,
    /// Save chip select line is held low between bytes
    spi_selected: bool,

    /// Encryption seed 0
    encrypt_seed0: u64,
    /// Encryption seed 1
    encrypt_seed1: u64,

    /// Keycode for encryption (3 words)
    pub keycode: [u32; 3],

    /// SD card image for homebrew (host data, see [`sd`]).
    #[serde(skip)]
    pub sd: Option<sd::SdCard>,
}

impl Default for NDSCart {
    fn default() -> Self {
        Self::new()
    }
}

impl NDSCart {
    /// Create new cartridge controller
    pub fn new() -> Self {
        Self {
            // DS Encryption by Gamecode/Idcode (KEY1): https://problemkaputt.de/gbatek.htm#dsencryptionbygamecodeidcodekey1
            key1_buffer: vec![0_u8; 0x1048], // KEY1 table: 1048h bytes copied from ARM7 BIOS offset 30h
            cmd_encrypt_mode: 0,
            rom: Vec::new(),
            save_database: Vec::new(),
            rom_name: String::new(),
            backup: BackupMemory::default(),
            database_size: 0,
            rom_size: 0,
            command_buffer: [0_u8; 8],
            data_output: 0,
            rom_data_index: 0,
            command_id: CartCommand::Empty,
            secure_area_index: 0,
            cycles_left: 0,
            bytes_left: 0,
            romctrl: RegRomCtrl::new(),
            auxspicnt: 0,
            spi_data: 0,
            spi_selected: false,
            encrypt_seed0: 0,
            encrypt_seed1: 0,
            keycode: [0_u32; 3],
            sd: None,
        }
    }

    /// The loaded ROM's identity (see [`RomFingerprint`]).
    pub fn rom_fingerprint(&self) -> RomFingerprint {
        RomFingerprint::of_rom(&self.rom)
    }

    /// Resets cartridge state to power-on defaults.
    ///
    /// The loaded ROM and the backup memory contents are kept.
    pub const fn power_on(&mut self) {
        self.cycles_left = 8;
        self.bytes_left = 0;
        self.romctrl.word_ready = true;
        self.romctrl.block_busy = false;
        self.cmd_encrypt_mode = 0;

        self.auxspicnt = 0;
        self.spi_data = 0;
        self.spi_selected = false;
        self.backup.reset();
    }

    /// Loads save database file into memory.
    ///
    /// # Errors
    pub fn load_database(&mut self, file_name: &Path) -> Result<(), CartridgeError> {
        use std::{fs::File, io::Read};

        self.save_database.clear();
        self.database_size = 0;

        // #[cfg(feature = "log")]
        // log::info!("Loading save database: {}", file_name.display());

        let mut file = File::open(file_name).map_err(|source| {
            #[cfg(feature = "log")]
            log::error!("Failed to open save database: {}", file_name.display());

            CartridgeError::OpenSaveDatabase { path: file_name.to_path_buf(), source }
        })?;

        let metadata = std::fs::metadata(file_name).map_err(|source| {
            #[cfg(feature = "log")]
            log::error!("Failed to read metadata: {}", file_name.display());

            CartridgeError::MetadataSaveDatabase { path: file_name.to_path_buf(), source }
        })?;

        let size = metadata.len();
        self.database_size = size;

        if size % 19 != 0 {
            #[cfg(feature = "log")]
            log::error!("Invalid save database format: {} size={size}", file_name.display(),);

            return Err(CartridgeError::InvalidSaveDatabaseFormat {
                path: file_name.to_path_buf(),
                size,
            });
        }

        self.save_database.resize(size as usize, 0);

        file.read_exact(&mut self.save_database).map_err(|source| {
            #[cfg(feature = "log")]
            log::error!("Failed to read save database contents: {}", file_name.display());

            CartridgeError::ReadSaveDatabase { path: file_name.to_path_buf(), source }
        })?;

        // #[cfg(feature = "log")]
        // log::info!("Save database successfully loaded: {}", file_name.display());

        Ok(())
    }

    // Move to struct Emulator
    // Loads ROM image and optional save data.
    // pub fn load_rom( &mut self, file_name: &Path, is_direct_boot_enabled: bool,) -> Result<(), CartridgeError>;

    /// Writes save data to disk if modified.
    ///
    /// # Errors
    ///
    /// Called once per frame; the file is only written after the game has
    /// stopped writing for a short while (see [`BackupMemory::end_frame`]).
    pub fn save_check(&mut self) -> Result<(), CartridgeError> {
        self.backup.end_frame().map_err(|source| self.save_error(source))
    }

    /// Writes pending save data to disk immediately (e.g. on exit).
    ///
    /// # Errors
    pub fn flush_save(&mut self) -> Result<(), CartridgeError> {
        self.backup.flush().map_err(|source| self.save_error(source))
    }

    /// Wraps an I/O error with the save file path.
    fn save_error(&self, source: std::io::Error) -> CartridgeError {
        CartridgeError::SaveWrite {
            path: self.backup.path().map(Path::to_path_buf).unwrap_or_default(),
            source,
        }
    }
}
