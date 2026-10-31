//! SD card for homebrew (melonDS `CartHomebrew` / `CartSD`).
//!
//! ```text
//!  homebrew ARM9 binary with a DLDI stub ("\xED\xA5\x8D\xBF Chishm")
//!     | apply_dldi(): replaced by the melonDS driver (dldi_driver.rs)
//!     v
//!  driver: C0h sector  -> 512 bytes read as 128 words from 0x04100010
//!          C1h sector  -> 128 words written to 0x04100010
//!     |
//!     v
//!  SdCard: a raw FAT disk image file on the host, 512-byte sectors
//! ```
use std::{
    fs::File,
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    path::Path,
};

use crate::dldi_driver::MELON_DLDI;

const SECTOR: usize = 512;
/// DLDI header magic: `ED A5 8D BF`, `" Chishm\0"`.
const DLDI_MAGIC: [u32; 3] = [0xBF8D_A5ED, 0x6968_4320, 0x006D_6873];

/// Disk image plus the sector being transferred.
#[derive(Debug)]
pub struct SdCard {
    file: File,
    read_only: bool,
    sector: u32,
    pos: usize,
    buf: [u8; SECTOR],
}

impl SdCard {
    /// Opens a disk image (read-only when the file is, or when asked).
    ///
    /// # Errors
    /// The image cannot be opened.
    pub fn open(path: &Path, read_only: bool) -> std::io::Result<Self> {
        let (file, read_only) =
            match std::fs::OpenOptions::new().read(true).write(!read_only).open(path) {
                Ok(f) => (f, read_only),
                Err(_) if !read_only => (File::open(path)?, true),
                Err(e) => return Err(e),
            };
        Ok(Self { file, read_only, sector: 0, pos: 0, buf: [0; SECTOR] })
    }

    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn load(&mut self) {
        self.buf.fill(0);
        let off = u64::from(self.sector) * SECTOR as u64;
        if self.file.seek(SeekFrom::Start(off)).is_ok() {
            // Past the end of the image reads as zeros.
            let _ = self.file.read(&mut self.buf);
        }
    }

    /// Command C0h (read) or C1h (write) at `sector`.
    pub fn start(&mut self, cmd: u8, sector: u32) {
        self.sector = sector;
        self.pos = 0;
        if cmd == 0xC0 {
            self.load();
        }
    }

    /// Next word of a read.
    pub fn receive(&mut self) -> u32 {
        let b = &self.buf[self.pos..self.pos + 4];
        let w = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        self.pos += 4;
        if self.pos >= SECTOR {
            self.sector += 1;
            self.pos = 0;
            self.load();
        }
        w
    }

    /// Next word of a write; a full sector goes to the image.
    pub fn transmit(&mut self, value: u32) {
        self.buf[self.pos..self.pos + 4].copy_from_slice(&value.to_le_bytes());
        self.pos += 4;
        if self.pos >= SECTOR {
            if !self.read_only {
                let off = u64::from(self.sector) * SECTOR as u64;
                if self.file.seek(SeekFrom::Start(off)).is_ok() {
                    let _ = self.file.write_all(&self.buf);
                }
            }
            self.sector += 1;
            self.pos = 0;
        }
    }
}

fn rd(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn wr(b: &mut [u8], i: usize, v: u32) {
    if let Some(s) = b.get_mut(i..i + 4) {
        s.copy_from_slice(&v.to_le_bytes());
    }
}

/// Replaces every DLDI stub in the ARM9 binary of `rom` with the melonDS
/// driver, relocated to the stub's address (melonDS
/// `CartSD::ApplyDLDIPatch`). Returns how many were patched.
pub fn apply_dldi(rom: &mut [u8], read_only: bool) -> usize {
    let patch = &MELON_DLDI[..];
    let (offset, size) = (rd(rom, 0x20) as usize, rd(rom, 0x2C) as usize);
    let end = (offset + size).min(rom.len());
    let Some(bin) = rom.get_mut(offset..end) else { return 0 };
    let is_magic = |b: &[u8], i: usize| (0..3).all(|k| rd(b, i + k * 4) == DLDI_MAGIC[k]);

    let mut count = 0;
    let mut i = 0;
    while i + 0x80 <= bin.len() {
        if !is_magic(bin, i) || bin.len() < i + patch.len() {
            i += 1;
            continue;
        }
        // The stub's size class (0Fh) must hold the driver's (0Dh).
        if patch[0x0D] > bin[i + 0x0F] {
            i += patch.len();
            continue;
        }
        let mut memaddr = rd(bin, i + 0x40);
        if memaddr == 0 {
            memaddr = rd(bin, i + 0x68).wrapping_sub(0x80);
        }
        let base = rd(patch, 0x40);
        let delta = memaddr.wrapping_sub(base);
        let patch_end = base + (1 << patch[0x0D]);

        bin[i..i + patch.len()].copy_from_slice(patch);
        for f in (0x40..=0x5C).step_by(4).chain((0x68..=0x7C).step_by(4)) {
            let v = rd(bin, i + f).wrapping_add(delta);
            wr(bin, i + f, v);
        }
        // Fix sections: 01 all, 02 GOT, 04 constructors -> relocate
        // pointers into the driver; 08 BSS -> zero.
        let fix = patch[0x0E];
        for (bit, start_field) in [(1, 0x40), (2, 0x48), (4, 0x50)] {
            if fix & bit == 0 {
                continue;
            }
            let (s, e) = (rd(patch, start_field) - base, rd(patch, start_field + 4) - base);
            for a in (s..e).step_by(4) {
                let v = rd(bin, i + a as usize);
                if (base..patch_end).contains(&v) {
                    wr(bin, i + a as usize, v.wrapping_add(delta));
                }
            }
        }
        if fix & 8 != 0 {
            let (s, e) = ((rd(patch, 0x58) - base) as usize, (rd(patch, 0x5C) - base) as usize);
            if let Some(bss) = bin.get_mut(i + s..i + e) {
                bss.fill(0);
            }
        }
        if read_only {
            bin[i + 0x64] &= !0x02; // feature flag: can write
            let ws = (rd(bin, i + 0x74).wrapping_sub(memaddr)) as usize + i;
            wr(bin, ws, 0xE3A0_0000); // mov r0, #0
            wr(bin, ws + 4, 0xE12F_FF1E); // bx lr
        }
        count += 1;
        i += patch.len();
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_a_dldi_stub_and_relocates_it() {
        let mut rom = vec![0_u8; 0x4000 + 0x8000];
        wr(&mut rom, 0x20, 0x4000); // ARM9 ROM offset
        wr(&mut rom, 0x2C, 0x8000); // ARM9 size
        let stub = 0x4000 + 0x100;
        for (k, m) in DLDI_MAGIC.iter().enumerate() {
            wr(&mut rom, stub + k * 4, *m);
        }
        rom[stub + 0x0F] = 0x0F; // 32 KiB stub
        wr(&mut rom, stub + 0x40, 0x0238_0000); // where the stub lives
        assert_eq!(apply_dldi(&mut rom, false), 1);
        assert_eq!(&rom[stub + 0x10..stub + 0x16], b"melonD");
        assert_eq!(rd(&rom, stub + 0x40), 0x0238_0000);
    }

    #[test]
    fn sd_round_trip() {
        let dir = std::env::temp_dir().join("lunaris_sd_test.img");
        std::fs::write(&dir, vec![0_u8; 4 * SECTOR]).unwrap();
        let mut sd = SdCard::open(&dir, false).unwrap();
        sd.start(0xC1, 2);
        for i in 0..128 {
            sd.transmit(i);
        }
        sd.start(0xC0, 2);
        assert_eq!(sd.receive(), 0);
        assert_eq!(sd.receive(), 1);
        drop(sd);
        let _ = std::fs::remove_file(dir);
    }
}
