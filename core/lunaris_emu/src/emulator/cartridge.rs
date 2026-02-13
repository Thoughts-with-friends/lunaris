use std::path::Path;

use crate::Emulator;
use crate::cartridge::{CartCommand, CartridgeError};

impl Emulator {
    /// Loads ROM image and optional save data.
    pub fn cartridge_load_rom(&mut self, rom_file: &Path) -> Result<(), CartridgeError> {
        use std::fs::File;
        use std::io::Read as _;

        self.power_on();

        #[cfg(feature = "tracing")]
        tracing::info!("Loading ROM {}", rom_file.display());

        let mut file = File::open(rom_file).map_err(|source| {
            #[cfg(feature = "tracing")]
            tracing::error!("Failed to load {}", rom_file.display());

            CartridgeError::OpenRom {
                path: rom_file.to_path_buf(),
                source,
            }
        })?;

        let metadata =
            std::fs::metadata(rom_file).map_err(|source| CartridgeError::MetadataRom {
                path: rom_file.to_path_buf(),
                source,
            })?;

        let rom_size = metadata.len();
        self.cart.rom_size = rom_size;

        #[cfg(feature = "tracing")]
        tracing::info!("Allocating {} bytes for ROM", rom_size);

        self.cart.rom.resize(rom_size as usize, 0);

        #[cfg(feature = "tracing")]
        tracing::info!("Loading ROM into memory");

        file.read_exact(&mut self.cart.rom)
            .map_err(|source| CartridgeError::ReadRom {
                path: rom_file.to_path_buf(),
                source,
            })?;

        // Remove extension
        if let Some(stem) = rom_file.file_stem() {
            self.cart.rom_name = stem.to_string_lossy().to_string();
        }

        // Try load save
        let save_path = rom_file.with_extension("sav");

        if let Ok(mut save_file) = File::open(&save_path) {
            let meta =
                std::fs::metadata(&save_path).map_err(|source| CartridgeError::ReadSave {
                    path: save_path.clone(),
                    source,
                })?;

            let file_size = meta.len() as usize;

            if file_size > 0 {
                save_file
                    .read_exact(&mut self.cart.spi_save[..file_size])
                    .map_err(|source| CartridgeError::ReadSave {
                        path: save_path.clone(),
                        source,
                    })?;

                self.cart.save_size = file_size;

                #[cfg(feature = "tracing")]
                tracing::info!(
                    "Loaded save {} size {}",
                    save_path.display(),
                    self.cart.save_size
                );
            }
        } else {
            // Database fallback
            if !self.cart.save_database.is_empty() {
                let entries = self.cart.database_size as usize / 19;

                for index in 0..entries {
                    let base = index * 19;

                    let mut match_ok = true;

                    for c in 0..16 {
                        if self.cart.rom[c] != self.cart.save_database[base + c] {
                            match_ok = false;
                            break;
                        }
                    }

                    if match_ok {
                        #[cfg(feature = "tracing")]
                        tracing::info!("Found ROM entry in database");

                        let size_index = self.cart.save_database[base + 18];

                        self.cart.save_size = match size_index {
                            0x02 => 512,
                            0x03 => 1024 * 8,
                            0x04 => 1024 * 64,
                            0x05 => 1024 * 256,
                            0x06 => 1024 * 512,
                            0x07 => 1024 * 1024,
                            _ => {
                                #[cfg(feature = "tracing")]
                                tracing::error!("Unrecognized save format {}", size_index);
                                0
                            }
                        };

                        #[cfg(feature = "tracing")]
                        tracing::info!("Save size {}", self.cart.save_size);
                    }
                }
            }
        }

        if self.cart.save_size == 512 {
            self.cart.save_type = 0;
        } else if (1024 * 8..=1024 * 64).contains(&self.cart.save_size) {
            self.cart.save_type = 1;
        } else if self.cart.save_size > 1024 * 64 {
            self.cart.save_type = 2;
        }

        if self.config.direct_boot_enabled {
            return Ok(());
        }

        // Safe little-endian reads
        let read_u32 = |buf: &[u8], offset: usize| -> u32 {
            u32::from_le_bytes([
                buf[offset],
                buf[offset + 1],
                buf[offset + 2],
                buf[offset + 3],
            ])
        };

        let game_code = read_u32(&self.cart.rom, 0xC);
        let arm_rom_base = read_u32(&self.cart.rom, 0x20) as usize;

        if (0x4000..0x8000).contains(&arm_rom_base) {
            let sig0 = read_u32(&self.cart.rom, arm_rom_base);
            let sig1 = read_u32(&self.cart.rom, arm_rom_base + 0x10);

            if sig0 == 0xE7FFDEFF && sig1 != 0xE7FFDEFF {
                #[cfg(feature = "tracing")]
                tracing::info!("Encrypting secure area");

                self.cart.rom[arm_rom_base..arm_rom_base + 8].copy_from_slice(b"encryObj");

                self.cartridge_init_keycode(game_code, 3, 2);

                for i in (0..0x800).step_by(8) {
                    let ptr = arm_rom_base + i;
                    let block = [
                        read_u32(&self.cart.rom, ptr),
                        read_u32(&self.cart.rom, ptr + 4),
                    ];
                    let data = self.cart.key1_encrypt(block[0], block[1]);
                    self.cart.rom[ptr..ptr + 4].copy_from_slice(&data[0].to_le_bytes());
                    self.cart.rom[ptr + 4..ptr + 8].copy_from_slice(&data[1].to_le_bytes());
                }

                self.cartridge_init_keycode(game_code, 2, 2);

                let ptr = arm_rom_base;
                let block = [
                    read_u32(&self.cart.rom, ptr),
                    read_u32(&self.cart.rom, ptr + 4),
                ];
                let data = self.cart.key1_encrypt(block[0], block[1]);
                self.cart.rom[ptr..ptr + 4].copy_from_slice(&data[0].to_le_bytes());
                self.cart.rom[ptr + 4..ptr + 8].copy_from_slice(&data[1].to_le_bytes());
            }
        }

        self.cartridge_init_keycode(game_code, 2, 2);

        Ok(())
    }

    /// Initializes keycode from chip ID and level.
    pub fn cartridge_init_keycode(&mut self, idcode: u32, level: i32, modulo: u32) {
        if let Some(bios) = &self.arm7_bios.get(0x30..0x30 + 0x1048) {
            self.cart.key1_buffer[..0x1048].copy_from_slice(bios);
        } else {
            #[cfg(feature = "tracing")]
            tracing::warn!("arm7_bios need: 0x30..0x30 + 0x1048");
        }

        self.cart.keycode[0] = idcode;
        self.cart.keycode[1] = idcode >> 1;
        self.cart.keycode[2] = idcode << 1;
        if level >= 1 {
            self.cart.apply_keycode(modulo);
        }
        if level >= 2 {
            self.cart.apply_keycode(modulo);
        }
        self.cart.keycode[1] <<= 1;
        self.cart.keycode[2] >>= 1;
        if level >= 3 {
            self.cart.apply_keycode(modulo);
        }
    }

    /// Executes cartridge transfer logic for a given number of cycles.
    #[cfg_attr(feature = "tracing", tracing::instrument)]
    pub fn cartridge_run(&mut self, cycles: i32) {
        if self.cart.romctrl.block_busy && !self.cart.romctrl.word_ready {
            self.cart.cycles_left -= cycles;

            if self.cart.cycles_left > 0 {
                return;
            }

            self.cart.cycles_left = 8;

            match self.cart.command_id {
                CartCommand::Dummy => {
                    self.cart.data_output = 0xFFFF_FFFF;
                }

                CartCommand::GetHeader => {
                    self.cart.data_output =
                        self.cart.direct_read_word(self.cart.rom_data_index as u32);
                    self.cart.rom_data_index += 4;

                    if self.cart.rom_data_index > 0xFFF {
                        self.cart.rom_data_index = 0;
                    }

                    self.cart.romctrl.word_ready = true;
                }

                CartCommand::GetChipId => {
                    // Macronix 64MB ROM compatible ID
                    self.cart.data_output = 0x0000_3FC2;
                    self.cart.romctrl.word_ready = true;
                }

                CartCommand::EnableKey1 => {
                    self.cart.cmd_encrypt_mode = 1;
                }

                CartCommand::GetSecureAreaBlock => {
                    self.cart.data_output = self.cart.direct_read_word(self.cart.secure_area_index);
                    self.cart.secure_area_index += 4;
                    self.cart.romctrl.word_ready = true;
                }

                CartCommand::ReadRom => {
                    if self.cart.rom_data_index < 0x8000 {
                        let addr = 0x8000 + (self.cart.rom_data_index & 0x1FF);
                        self.cart.data_output = self.cart.direct_read_word(addr as u32);
                    } else {
                        self.cart.data_output =
                            self.cart.direct_read_word(self.cart.rom_data_index as u32);
                    }

                    self.cart.rom_data_index += 4;
                    self.cart.romctrl.word_ready = true;
                }

                _ => {
                    #[cfg(feature = "tracing")]
                    tracing::error!("Unknown cart command: {:02X?}", self.cart.command_buffer);

                    #[cfg(not(feature = "tracing"))]
                    eprintln!("Unknown cart command: {:02X?}", self.cart.command_buffer);
                }
            }

            self.cart.bytes_left -= 4;

            self.gamecart_dma_request();

            if self.cart.bytes_left <= 0 {
                self.cart.romctrl.block_busy = false;

                if self.cart.auxspicnt.irq_after_transfer {
                    if self.arm7_has_cart_rights() {
                        self.request_interrupt7(crate::interrupts::Interrupt::CartTransfer);
                    } else {
                        self.request_interrupt9(crate::interrupts::Interrupt::CartTransfer);
                    }
                }
            }
        }
    }
}
