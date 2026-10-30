//! EEPROM / FRAM backup chips: tiny EEPROM (512 bytes, 1 address byte)
//! and EEPROM / FRAM (2 or 3 address bytes). FRAM speaks the same protocol.
use super::BackupMemory;

impl BackupMemory {
    /// 512-byte EEPROM: `cmd | 8` addresses the upper half.
    pub(super) fn eeprom_tiny(&mut self, value: u8) -> u8 {
        // DS Cartridge Backup: https://problemkaputt.de/gbatek.htm#dscartridgebackup
        let high = if self.cmd & 0x08 != 0 { 0x100 } else { 0 }; // 0.5K EEPROM: command bit 3 = address bit 8
        match self.cmd {
            // Write status register (only the block protect bits).
            0x01 => {
                if self.pos == 1 {
                    self.status = (self.status & 0x01) | (value & 0x0C); // WRSR: status bits 2-3 (block protect), keep WIP (bit 0)
                }
                0xFF // data bus idle
            }
            // Read status register (unused bits read as 1).
            0x05 => self.status | 0xF0,
            // Write low / high.
            0x02 | 0x0A => {
                if self.pos < 2 {
                    self.addr = value as u32;
                } else {
                    if self.write_enabled() {
                        self.data[((self.addr + high) & 0x1FF) as usize] = value; // 512-byte address space
                        self.mark_written();
                    }
                    self.addr += 1;
                }
                0xFF // data bus idle
            }
            // Read low / high.
            0x03 | 0x0B => {
                if self.pos < 2 {
                    self.addr = value as u32;
                    0xFF // data bus idle
                } else {
                    let ret = self.data[((self.addr + high) & 0x1FF) as usize]; // 512-byte address space
                    self.addr += 1;
                    ret
                }
            }
            _ => 0xFF, // data bus idle
        }
    }

    /// 8 KiB .. 128 KiB EEPROM / FRAM.
    pub(super) fn eeprom(&mut self, value: u8) -> u8 {
        let addr_bytes = if self.data.len() > 0x1_0000 { 3 } else { 2 }; // chips larger than 64K use 3 address bytes
        let mask = self.data.len() as u32 - 1;
        match self.cmd {
            0x01 => {
                // 01h: WRSR (write status register)
                if self.pos == 1 {
                    self.status = (self.status & 0x01) | (value & 0x0C); // status bits 2-3 (block protect), keep WIP (bit 0)
                }
                0xFF // data bus idle
            }
            0x05 => self.status, // 05h: RDSR (read status register)
            0x02 => {
                // 02h: WRITE
                if self.pos <= addr_bytes {
                    self.addr = (self.addr << 8) | value as u32;
                } else {
                    if self.write_enabled() {
                        self.data[(self.addr & mask) as usize] = value;
                        self.mark_written();
                    }
                    self.addr += 1;
                }
                0xFF // data bus idle
            }
            0x03 => {
                // 03h: READ
                if self.pos <= addr_bytes {
                    self.addr = (self.addr << 8) | value as u32;
                    0xFF // data bus idle
                } else {
                    let ret = self.data[(self.addr & mask) as usize];
                    self.addr += 1;
                    ret
                }
            }
            _ => 0xFF, // data bus idle
        }
    }
}
