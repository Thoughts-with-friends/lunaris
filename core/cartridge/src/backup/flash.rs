//! FLASH backup chips (256 KiB - 1 MiB): 3 address bytes, page program
//! and page / sector erase.
use super::BackupMemory;

impl BackupMemory {
    /// SPI FLASH (ST M25PE / Sanyo LE25FW compatible command set).
    pub(super) fn flash(&mut self, value: u8) -> u8 {
        let mask = self.data.len() as u32 - 1;
        match self.cmd {
            0x05 => self.status,
            // Read JEDEC ID: ST Micro, then capacity (0x12 = 256K .. 0x14 = 1M).
            0x9F => match self.pos {
                // DS Cartridge Backup: https://problemkaputt.de/gbatek.htm#dscartridgebackup
                1 => 0x20, // RDID byte 1: manufacturer 20h (ST)
                2 => 0x40, // RDID byte 2: memory type 40h
                3 => 0x11 + (self.data.len() >> 18).trailing_zeros() as u8 + 1, // RDID byte 3: capacity code (13h = 512K)
                _ => 0xFF,                                                      // data bus idle
            },
            // Read / fast read (one dummy byte after the address).
            0x03 | 0x0B => {
                let dummy = u32::from(self.cmd == 0x0B); // 0Bh: FAST READ has a dummy byte
                if self.pos <= 3 {
                    self.addr = (self.addr << 8) | value as u32;
                    0xFF // data bus idle
                } else if self.pos <= 3 + dummy {
                    0xFF // data bus idle
                } else {
                    let ret = self.data[(self.addr & mask) as usize];
                    self.addr = self.addr.wrapping_add(1);
                    ret
                }
            }
            // Page write (erase + program) / page program (bits 1 -> 0 only).
            0x0A | 0x02 => {
                if self.pos <= 3 {
                    self.addr = (self.addr << 8) | value as u32;
                } else if self.write_enabled() {
                    let index = (self.addr & mask) as usize;
                    if self.cmd == 0x0A {
                        // 0Ah: PAGE WRITE (erase + program)
                        self.data[index] = value;
                    } else {
                        self.data[index] &= value;
                    }
                    self.mark_written();
                    // The address wraps within the 256-byte page.
                    self.addr = (self.addr & !0xFF) | (self.addr.wrapping_add(1) & 0xFF);
                }
                0xFF // data bus idle
            }
            // Page erase (256 B) / sector erase (64 KiB).
            0xDB | 0xD8 => {
                if self.pos <= 3 {
                    self.addr = (self.addr << 8) | value as u32;
                }
                if self.pos == 3 && self.write_enabled() {
                    let len: u32 = if self.cmd == 0xDB { 0x100 } else { 0x1_0000 }; // DBh: page erase (100h bytes), else sector erase (10000h bytes)
                    let start = (self.addr & mask & !(len - 1)) as usize;
                    let end = (start + len as usize).min(self.data.len());
                    self.data[start..end].fill(0xFF); // erased flash reads FFh
                    self.mark_written();
                }
                0xFF // data bus idle
            }
            // Chip erase.
            0xC7 => {
                if self.pos == 1 && self.write_enabled() {
                    self.data.fill(0xFF); // C7h: chip erase
                    self.mark_written();
                }
                0xFF // data bus idle
            }
            _ => 0xFF, // data bus idle
        }
    }
}
