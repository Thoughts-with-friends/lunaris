//! AUXSPI: the serial bus to the cartridge backup memory
//! (AUXSPICNT 0x040001A0, AUXSPIDATA 0x040001A2).
//! See GBATEK "DS Cartridge Backup".
use crate::NDSCart;

impl NDSCart {
    /// Returns AUXSPICNT register value.
    ///
    /// Transfers complete instantly, so the busy bit (7) never reads as set.
    pub const fn get_auxspicnt(&self) -> u16 {
        // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
        self.auxspicnt & !(1 << 7) // AUXSPICNT bit 7: SPI bus busy
    }

    /// Reads AUXSPIDATA: the byte the save chip sent back during the last
    /// transfer (0 while the slot or SPI mode is disabled).
    pub const fn read_auxspidata(&self) -> u8 {
        // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
        if self.auxspicnt & 0xA000 != 0xA000 {
            // AUXSPICNT bit 13 (SPI mode) and bit 15 (slot enable) must both be set
            return 0;
        }
        self.spi_data
    }

    /// Writes AUXSPICNT bits selected by `mask`.
    ///
    /// Clearing bit 13 (SPI mode) while the chip is selected releases the
    /// chip select line, setting it again re-asserts it (melonDS
    /// `NDSCartSlot::Interface::WriteSPICnt`).
    #[allow(clippy::missing_const_for_fn)]
    pub fn write_auxspicnt(&mut self, value: u16, mask: u16) {
        #[cfg(feature = "log")]
        log::trace!(target: "auxspi", "cnt {:04X} -> {value:04X}/{mask:04X} sel={}", self.auxspicnt, self.spi_selected);
        let value = value & mask;
        if self.spi_selected {
            // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
            if self.auxspicnt & !value & (1 << 13) != 0 {
                // AUXSPICNT bit 13: NDS slot mode (0=ROM, 1=SPI)
                self.backup.release();
                self.spi_selected = false;
            } else if !self.auxspicnt & value & (1 << 13) != 0 {
                // AUXSPICNT bit 13: NDS slot mode (0=ROM, 1=SPI)
                self.backup.select();
            }
        }
        self.auxspicnt = (self.auxspicnt & !mask) | (value & 0xE043); // AUXSPICNT writable bits: 0-1, 6, 13-15
    }

    /// Sets AUXSPICNT register value.
    pub fn set_auxspicnt(&mut self, value: u16) {
        self.write_auxspicnt(value, 0xFFFF); // full halfword
    }

    /// Sets low byte of AUXSPICNT register.
    pub fn set_lo_auxspicnt(&mut self, value: u8) {
        self.write_auxspicnt(value as u16, 0x00FF); // lower byte
    }

    /// Sets high byte of AUXSPICNT register.
    pub fn set_hi_auxspicnt(&mut self, value: u8) {
        self.write_auxspicnt((value as u16) << 8, 0xFF00); // upper byte
    }

    /// Writes AUXSPIDATA: exchanges one byte with the save chip.
    ///
    /// ```text
    /// from: ds_emu arm7_/arm9_write_halfword|byte(0x040001A2)
    ///   1. slot enabled and AUXSPICNT in SPI mode, else ignore
    ///   2. first byte -> backup.select()       (byte is a command)
    ///   3. backup.transfer(value)              EEPROM / FLASH state
    ///                                          machine (backup/), reply
    ///                                          kept for AUXSPIDATA reads
    ///   4. chip-select hold clear -> backup.release() (ends the command,
    ///      a finished write marks the save dirty)
    /// to:   save_check() writes the .sav after the game stops writing
    /// ```
    ///
    /// The chip is selected by the first byte and released after any byte
    /// written while AUXSPICNT bit 6 (chip select hold) is clear.
    pub fn set_auxspidata(&mut self, value: u8) {
        // Slot enabled and in SPI (backup) mode.
        if self.auxspicnt & 0xA000 != 0xA000 {
            return;
        }

        if !self.spi_selected {
            self.backup.select();
        }
        self.spi_data = self.backup.transfer(value);
        #[cfg(feature = "log")]
        // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
        log::trace!(target: "auxspi", "sel={} in={value:02X} out={:02X} hold={}", self.spi_selected, self.spi_data, self.auxspicnt & (1 << 6) != 0); // AUXSPICNT bit 6: SPI hold chipselect

        let hold = self.auxspicnt & (1 << 6) != 0; // AUXSPICNT bit 6: SPI hold chipselect
        if !hold {
            self.backup.release();
        }
        self.spi_selected = hold;
    }

    /// Whether a ROM transfer should raise the cartridge IRQ (AUXSPICNT bit 14).
    pub const fn transfer_irq_enabled(&self) -> bool {
        // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
        self.auxspicnt & (1 << 14) != 0 // AUXSPICNT bit 14: transfer ready IRQ enable
    }
}
