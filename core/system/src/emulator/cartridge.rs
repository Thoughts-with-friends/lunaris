use std::path::Path;

use ds_cartridge::{BackupMemory, CartCommand, CartridgeError, SaveMemType, game_db};

use crate::Emulator;

impl Emulator {
    /// Loads ROM image and optional save data.
    ///
    /// ```text
    /// from: load_rom() step 1
    ///   |
    ///   v
    /// cartridge_load_rom(path)
    ///   1. flush the previous game's save
    ///   2. read the whole ROM into cart.rom, remember its name
    ///   3. cartridge_setup_backup()   save chip type + .sav contents
    ///   4. direct boot?  done
    ///   5. otherwise re-encrypt the secure area with KEY1 (as on a real
    ///      card) and leave KEY1 initialised for the boot protocol
    ///   |
    ///   v
    /// to:   load_rom() -> power_on()
    /// ```
    /// # Errors
    pub fn cartridge_load_rom(&mut self, rom_file: &Path) -> Result<(), CartridgeError> {
        use std::{fs::File, io::Read as _};

        // #[cfg(feature = "log")]
        // log::info!("Loading ROM {}", rom_file.display());

        // Persist the previous game's save before it is replaced.
        if let Err(_err) = self.cart.flush_save() {
            #[cfg(feature = "log")]
            log::error!("{_err}");
        }

        let mut file = File::open(rom_file).map_err(|source| {
            // #[cfg(feature = "log")]
            // log::error!("Failed to load {}", rom_file.display());

            CartridgeError::OpenRom { path: rom_file.to_path_buf(), source }
        })?;

        let metadata = std::fs::metadata(rom_file).map_err(|source| {
            CartridgeError::MetadataRom { path: rom_file.to_path_buf(), source }
        })?;

        let rom_size = metadata.len();
        self.cart.rom_size = rom_size;

        // #[cfg(feature = "log")]
        // log::info!("Allocating {} bytes for ROM", rom_size);

        self.cart.rom.resize(rom_size as usize, 0);

        // #[cfg(feature = "log")]
        // log::info!("Loading ROM into memory");

        file.read_exact(&mut self.cart.rom)
            .map_err(|source| CartridgeError::ReadRom { path: rom_file.to_path_buf(), source })?;

        // Remove extension
        if let Some(stem) = rom_file.file_stem() {
            self.cart.rom_name = stem.to_string_lossy().to_string();
        }

        self.cartridge_setup_backup(rom_file)?;

        // Only re-encrypt the ROM if this is not a direct boot
        if self.config.direct_boot_enabled {
            return Ok(());
        }

        // Safe little-endian reads
        let read_u32 = |buf: &[u8], offset: usize| -> u32 {
            u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]])
        };

        // DS Cartridge Header: https://problemkaputt.de/gbatek.htm#dscartridgeheader
        let game_code = read_u32(&self.cart.rom, 0xC); // header 00Ch: gamecode
        let arm_rom_base = read_u32(&self.cart.rom, 0x20) as usize; // header 020h: ARM9 ROM offset

        if (0x4000..0x8000).contains(&arm_rom_base) {
            // ARM9 code inside the secure area (4000h-7FFFh)
            let sig0 = read_u32(&self.cart.rom, arm_rom_base);
            let sig1 = read_u32(&self.cart.rom, arm_rom_base + 0x10); // secure area +10h: signature

            if sig0 == 0xE7FFDEFF && sig1 != 0xE7FFDEFF {
                // #[cfg(feature = "log")]
                // log::info!("Encrypting secure area");

                self.cart.rom[arm_rom_base..arm_rom_base + 8].copy_from_slice(b"encryObj");

                self.cartridge_init_keycode(game_code, 3, 2);

                // DS Cartridge Secure Area: https://problemkaputt.de/gbatek.htm#dscartridgesecurearea
                for i in (0..0x800).step_by(8) {
                    // first 2K of the secure area, in 8-byte KEY1 blocks
                    let ptr = arm_rom_base + i;
                    let block = [read_u32(&self.cart.rom, ptr), read_u32(&self.cart.rom, ptr + 4)];
                    let data = self.cart.key1_encrypt(block[0], block[1]);
                    self.cart.rom[ptr..ptr + 4].copy_from_slice(&data[0].to_le_bytes());
                    self.cart.rom[ptr + 4..ptr + 8].copy_from_slice(&data[1].to_le_bytes());
                }

                self.cartridge_init_keycode(game_code, 2, 2);

                let ptr = arm_rom_base;
                let block = [read_u32(&self.cart.rom, ptr), read_u32(&self.cart.rom, ptr + 4)];
                let data = self.cart.key1_encrypt(block[0], block[1]);
                self.cart.rom[ptr..ptr + 4].copy_from_slice(&data[0].to_le_bytes());
                self.cart.rom[ptr + 4..ptr + 8].copy_from_slice(&data[1].to_le_bytes());
            }
        }

        self.cartridge_init_keycode(game_code, 2, 2);

        Ok(())
    }

    /// Picks the save chip for the loaded ROM and loads its `.sav` file.
    ///
    /// The chip type comes from, in order: the melonDS game database
    /// ([`ds_cartridge::game_db`]), a CorgiDS save database loaded
    /// with [`Self::load_save_database`], the size of an existing `.sav` file,
    /// and finally melonDS' default (none for homebrew, 64 KiB EEPROM
    /// otherwise). An existing save shorter or longer than the chip is padded
    /// with `0xFF` or truncated.
    ///
    /// The save lives next to the ROM (`<rom>.sav`) unless
    /// [`crate::emulator::emu_config::Config::save_dir`] is set.
    fn cartridge_setup_backup(&mut self, rom_file: &Path) -> Result<(), CartridgeError> {
        let save_path = match &self.config.save_dir {
            Some(dir) => dir.join(rom_file.with_extension("sav").file_name().unwrap_or_default()),
            None => rom_file.with_extension("sav"),
        };

        let existing = match std::fs::read(&save_path) {
            Ok(data) => Some(data),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(CartridgeError::ReadSave { path: save_path, source });
            }
        };

        let header = |offset: usize| {
            self.cart
                .rom
                .get(offset..offset + 4)
                .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let game_code = header(0x0C);
        // Same test as melonDS `NDSHeader::IsHomebrew`.
        let homebrew = header(0x20) < 0x4000 || game_code == 0x2323_2323;

        let mem_type: SaveMemType = game_db::lookup(game_code)
            .map(|(_, t)| t)
            // DS Cartridge Backup: https://problemkaputt.de/gbatek.htm#dscartridgebackup
            .filter(|&t| t != 0xFF) // FFh = unknown save type
            .or_else(|| self.corgids_database_type())
            .or_else(|| existing.as_ref().and_then(|d| BackupMemory::type_from_size(d.len())))
            .unwrap_or(if homebrew { 0 } else { 3 });

        self.cart.backup = BackupMemory::new(mem_type);
        // Game codes starting with 'I' are carts with an infrared port
        // (melonDS `NDSCartSlot::ParseROM`).
        self.cart.backup.set_infrared(game_code & 0xFF == u32::from(b'I')); // gamecode starting with 'I': infrared cartridge
        if let Some(data) = &existing {
            self.cart.backup.load_contents(data);
        }
        self.cart.backup.set_path(Some(save_path));

        #[cfg(feature = "log")]
        log::info!(
            "Save memory: game code {game_code:08X}, type {mem_type} ({:?}, {} bytes), file {}",
            self.cart.backup.chip(),
            self.cart.backup.size(),
            self.cart.backup.path().map_or_else(String::new, |p| p.display().to_string()),
        );
        Ok(())
    }

    /// Looks the ROM up in a CorgiDS save database (19-byte entries: 16 header
    /// bytes, then the save size code at offset 18).
    fn corgids_database_type(&self) -> Option<SaveMemType> {
        let header = self.cart.rom.get(..16)?;
        let entry = self
            .cart
            .save_database
            .as_chunks::<19>()
            .0
            .iter()
            .find(|entry| &entry[..16] == header)?;
        // CorgiDS codes: 2 = 512 B, 3 = 8K, 4 = 64K, 5 = 256K, 6 = 512K, 7 = 1M.
        match entry[18] {
            // DS Cartridge Backup: https://problemkaputt.de/gbatek.htm#dscartridgebackup
            0x02 => Some(1), // EEPROM 0.5K
            0x03 => Some(2), // EEPROM 8K
            0x04 => Some(3), // EEPROM 64K
            0x05 => Some(5), // FLASH 256K
            0x06 => Some(6), // FLASH 512K
            0x07 => Some(7), // FLASH 1M
            _ => None,
        }
    }

    /// Initializes the KEY1 key table from the ARM7 BIOS seed and the game
    /// code (GBATEK "init_keycode"), applying it `level` times.
    /// Called by cartridge_load_rom() and the card protocol.
    pub fn cartridge_init_keycode(&mut self, idcode: u32, level: i32, modulo: u32) {
        // DS Encryption by Gamecode/Idcode (KEY1): https://problemkaputt.de/gbatek.htm#dsencryptionbygamecodeidcodekey1
        if let Some(bios) = &self.arm7_bios.get(0x30..0x30 + 0x1048) {
            // KEY1 table: 1048h bytes at ARM7 BIOS offset 30h
            self.cart.key1_buffer[..0x1048].copy_from_slice(bios); // KEY1 table size
        } else {
            #[cfg(feature = "log")]
            log::warn!("arm7_bios need: 0x30..0x30 + 0x1048"); // KEY1 table location in the ARM7 BIOS
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

    /// Produces the next 32-bit word of a running ROM transfer.
    ///
    /// ```text
    /// from: run() step 2g, every slice; the transfer was started by a
    ///       ROMCTRL write (cart command in 0x040001A8..)
    ///   |
    ///   v
    /// cartridge_run(cycles)
    ///   1. busy and no word waiting?  count down 8 cycles per word
    ///   2. by command: dummy / header / chip ID / KEY1 enable /
    ///      secure area / ROM data (0xB7) -> data_output, word_ready
    ///   3. bytes_left -= 4; gamecart_dma_request()  (DMA may read it)
    ///   4. transfer finished -> cart_end_transfer()  (busy off, IRQ)
    ///   |
    ///   v
    /// to:   the CPU or a DMA reads 0x04100010 -> cart_read_data()
    /// ```
    pub fn cartridge_run(&mut self, cycles: i32) {
        // #[cfg(feature = "log")]
        // log::trace!(%self.cart.romctrl.block_busy,%self.cart.romctrl.word_ready);

        if self.cart.romctrl.block_busy && !self.cart.romctrl.word_ready {
            self.cart.cycles_left -= cycles;

            if self.cart.cycles_left > 0 {
                return;
            }

            self.cart.cycles_left = 8;
            let has_data = self.cart.bytes_left > 0;

            match self.cart.command_id {
                CartCommand::Dummy => {
                    // DS Cartridge Protocol: https://problemkaputt.de/gbatek.htm#dscartridgeprotocol
                    self.cart.data_output = 0xFFFF_FFFF; // data bus idle (no data)
                }

                CartCommand::GetHeader => {
                    self.cart.data_output =
                        self.cart.direct_read_word(self.cart.rom_data_index as u32);
                    self.cart.rom_data_index += 4;

                    if self.cart.rom_data_index > 0xFFF {
                        // a ROM read returns 1000h bytes at most per command
                        self.cart.rom_data_index = 0;
                    }

                    self.cart.romctrl.word_ready = true;
                }

                CartCommand::GetChipId => {
                    self.cart.data_output = self.cart.chip_id();
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
                    self.cart.data_output =
                        self.cart.read_rom_b7_word(self.cart.rom_data_index as u32);

                    // #[cfg(feature = "log")]
                    // log::debug!(
                    //     "ROM_READ [ROM_CMD] Addr: 0x{:08X} Value: 0x{:08X}",
                    //     self.cart.rom_data_index,
                    //     self.cart.data_output
                    // );

                    self.cart.rom_data_index += 4;
                    self.cart.romctrl.word_ready = true;
                }

                _ => {
                    #[cfg(feature = "log")]
                    log::error!("Unknown cart command: {:02X?}", self.cart.command_buffer);
                }
            }

            self.cart.bytes_left -= 4;

            self.gamecart_dma_request();

            // A transfer with data ends once its last word has been read
            // (see `cart_read_data`); one without data ends right away.
            if self.cart.bytes_left <= 0 && (!has_data || !self.cart.romctrl.word_ready) {
                self.cart_end_transfer();
            }
        }
    }

    /// Reads the card data port (0x04100010).
    ///
    /// ```text
    /// from: arm7_/arm9_read_word(0x04100010) (CPU poll or cart DMA)
    /// to:   cart.get_output() (clears word_ready so cartridge_run makes
    ///       the next word); last word -> cart_end_transfer()
    /// ```
    ///
    /// ROMCTRL stays busy (bit 31) until the last word of the block has been
    /// read, not merely made ready (melonDS `NDSCartSlot::ReadROMData`).
    /// Polling loops check "word ready" and then "busy"; clearing busy early
    /// made them drop the final word, which the anti-piracy code of later
    /// games (e.g. Pokemon + Nobunaga's Ambition) detects.
    pub fn cart_read_data(&mut self) -> u32 {
        let was_ready = self.cart.romctrl.word_ready;
        let data = self.cart.get_output();
        if was_ready && self.cart.romctrl.block_busy && self.cart.bytes_left <= 0 {
            self.cart_end_transfer();
        }
        data
    }

    /// Clears ROMCTRL busy and raises the transfer-complete IRQ if enabled,
    /// on the CPU that owns the slot (from cartridge_run / cart_read_data).
    fn cart_end_transfer(&mut self) {
        self.cart.romctrl.block_busy = false;

        if self.cart.transfer_irq_enabled() {
            if self.arm7_has_cart_rights() {
                self.request_interrupt7(ds_interrupt::Interrupt::CartTransfer);
            } else {
                self.request_interrupt9(ds_interrupt::Interrupt::CartTransfer);
            }
        }
    }
}
