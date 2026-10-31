//! Slot-2 add-ons, plugged in with <code>[crate::Emulator::slot2].insert(..)</code>.
//!
//! ```text
//!  0x0800_0000 .. 0x09FF_FFFF  ROM bus (16-bit)     0x0A00_0000 .. 0x0A00_FFFF  SRAM bus (8-bit)
//!
//!  GbaCart        GBA game ROM + its SRAM / FLASH save (Pal Park style "dual slot")
//!  RumblePak      reads FFFDh (detection); any write toggles the motor
//!  RamExpansion   8 MiB RAM at 0x0900_0000, enable at 0x0824_0000 (Opera browser)
//!  GuitarGrip     reads F9FFh; SRAM byte = the four fret buttons (active low)
//! ```
//!
//! Ports of melonDS `GBACart::CartGame`, `CartRumblePak`,
//! `CartRAMExpansion` and `CartGuitarGrip`. GBATEK:
//! [DS Cart Expansion](https://problemkaputt.de/gbatek.htm#dscartexpansion)
use std::path::PathBuf;

use crate::slot2::{Slot2Device, Width};

/// First address of the 8-bit SRAM bus.
const SRAM_START: u32 = 0x0A00_0000;
/// Frames without save writes after which a GBA save is written out.
const FLUSH_DELAY_FRAMES: u32 = 30;

/// Reads `width` from a 16-bit bus whose halfwords come from `half`.
fn read_16bit_bus(addr: u32, width: Width, mut half: impl FnMut(u32) -> u16) -> u32 {
    match width {
        Width::Byte => u32::from(half(addr & !1) >> ((addr & 1) * 8)) & 0xFF,
        Width::Half => u32::from(half(addr & !1)),
        Width::Word => u32::from(half(addr & !3)) | (u32::from(half((addr & !3) + 2)) << 16),
    }
}

/// Reads `width` from the 8-bit SRAM bus (the byte repeats on wider reads).
fn read_8bit_bus(width: Width, byte: u8) -> u32 {
    let b = u32::from(byte);
    match width {
        Width::Byte => b,
        Width::Half => b * 0x0101,
        Width::Word => b * 0x0101_0101,
    }
}

/// Save chip of a GBA cartridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GbaSaveType {
    None,
    /// 32 KiB battery-backed SRAM.
    Sram,
    /// 64 KiB FLASH (Panasonic ID).
    Flash64K,
    /// 128 KiB FLASH (Sanyo ID, two banks).
    Flash128K,
}

impl GbaSaveType {
    /// From a save file length (melonDS `CartGame::SetupSave`; 128K + 16
    /// is an mGBA save with RTC data).
    pub const fn from_len(len: usize) -> Self {
        match len {
            0x8000 => Self::Sram,
            0x1_0000 => Self::Flash64K,
            0x2_0000 | 0x2_0010 => Self::Flash128K,
            _ => Self::None,
        }
    }

    /// From the library ID string the GBA SDK leaves in the ROM.
    pub fn detect(rom: &[u8]) -> Self {
        let has = |s: &[u8]| rom.windows(s.len()).any(|w| w == s);
        if has(b"FLASH1M_V") {
            Self::Flash128K
        } else if has(b"FLASH512_V") || has(b"FLASH_V") {
            Self::Flash64K
        } else if has(b"SRAM_V") || has(b"SRAM_F_V") {
            Self::Sram
        } else {
            Self::None
        }
    }

    const fn size(self) -> usize {
        match self {
            Self::None => 0,
            Self::Sram => 0x8000,
            Self::Flash64K => 0x1_0000,
            Self::Flash128K => 0x2_0000,
        }
    }
}

/// A GBA game cartridge in slot 2.
pub struct GbaCart {
    rom: Vec<u8>,
    save_type: GbaSaveType,
    save: Vec<u8>,
    path: Option<PathBuf>,
    dirty: bool,
    idle_frames: u32,
    // FLASH command state (melonDS SRAMFlashState)
    flash_state: u8,
    flash_cmd: u8,
    flash_bank: u8,
}

impl GbaCart {
    /// A cartridge with `rom` and, if given, the save file at `save_path`
    /// (loaded if it exists, written back while running). The save chip
    /// comes from the file size or else from the ROM's library string.
    pub fn new(rom: Vec<u8>, save_path: Option<PathBuf>) -> Self {
        let existing = save_path.as_ref().and_then(|p| std::fs::read(p).ok());
        let save_type = existing
            .as_ref()
            .map(|d| GbaSaveType::from_len(d.len()))
            .filter(|&t| t != GbaSaveType::None)
            .unwrap_or_else(|| GbaSaveType::detect(&rom));
        let mut save = vec![0xFF; save_type.size()];
        if let Some(d) = existing {
            let n = d.len().min(save.len());
            save[..n].copy_from_slice(&d[..n]);
        }
        Self {
            rom,
            save_type,
            save,
            path: save_path,
            dirty: false,
            idle_frames: 0,
            flash_state: 0,
            flash_cmd: 0,
            flash_bank: 0,
        }
    }

    pub const fn save_type(&self) -> GbaSaveType {
        self.save_type
    }

    pub fn save_data(&self) -> &[u8] {
        &self.save
    }

    /// Writes the save file now if it changed.
    /// # Errors
    /// Returns an error if writing the save file fails.
    pub fn flush(&mut self) -> std::io::Result<()> {
        if let (true, Some(path)) = (self.dirty, &self.path) {
            std::fs::write(path, &self.save)?;
        }
        self.dirty = false;
        Ok(())
    }

    fn rom_half(&self, addr: u32) -> u16 {
        let off = (addr & 0x01FF_FFFF) as usize;
        match self.rom.get(off..off + 2) {
            Some(b) => u16::from_le_bytes([b[0], b[1]]),
            // Past the end the ROM bus returns its own address bits.
            None => ((addr >> 1) & 0xFFFF) as u16,
        }
    }

    fn save_write(&mut self, offset: usize, value: u8) {
        if let Some(b) = self.save.get_mut(offset)
            && *b != value
        {
            *b = value;
            self.dirty = true;
            self.idle_frames = 0;
        }
    }

    fn sram_read(&self, addr: u32) -> u8 {
        let a = (addr & 0xFFFF) as usize;
        match self.save_type {
            GbaSaveType::Sram => self.save.get(a).copied().unwrap_or(0xFF),
            GbaSaveType::Flash64K | GbaSaveType::Flash128K => match self.flash_cmd {
                0x90 => match (a, self.save_type) {
                    (0, GbaSaveType::Flash64K) => 0x32, // Panasonic
                    (1, GbaSaveType::Flash64K) => 0x1B,
                    (0, _) => 0x62, // Sanyo
                    (1, _) => 0x13,
                    _ => 0xFF,
                },
                0 | 0xA0 | 0xB0 => self
                    .save
                    .get(a + 0x1_0000 * usize::from(self.flash_bank))
                    .copied()
                    .unwrap_or(0xFF),
                _ => 0xFF,
            },
            GbaSaveType::None => 0xFF,
        }
    }

    /// FLASH command sequences: AA @5555, 55 @2AAA, command @5555.
    fn flash_write(&mut self, a: u32, v: u8) {
        let bank_off = 0x1_0000 * usize::from(self.flash_bank);
        match (self.flash_state, a, v) {
            (0, 0x5555, 0xF0) | (0x92, ..) => (self.flash_state, self.flash_cmd) = (0, 0),
            (0 | 0x90, 0x5555, 0xAA) | (1 | 0x91, 0x2AAA, 0x55) => {
                self.flash_state += 1;
            }
            (0, 0x0000, _) if self.flash_cmd == 0xB0 => {
                self.flash_bank = v & 1; // bank switch (128K)
                self.flash_cmd = 0;
            }
            (2, 0x5555, cmd) => {
                self.flash_state = if matches!(cmd, 0x80 | 0x90) { cmd } else { 0 };
                self.flash_cmd = cmd;
            }
            (0x80, 0x5555, 0xAA) => self.flash_state = 0x81,
            (0x81, 0x2AAA, 0x55) => self.flash_state = 0x82,
            (0x82, _, 0x10) => {
                // chip erase
                self.save.fill(0xFF);
                self.dirty = true;
                (self.flash_state, self.flash_cmd) = (0, 0);
            }
            (0x82, _, 0x30) => {
                // 4 KiB sector erase
                let start = (bank_off + (a as usize & 0xF000)).min(self.save.len());
                let end = (start + 0x1000).min(self.save.len());
                self.save[start..end].fill(0xFF);
                self.dirty = true;
                (self.flash_state, self.flash_cmd) = (0, 0);
            }
            _ if self.flash_cmd == 0xA0 => {
                self.save_write(bank_off + a as usize, v); // program one byte
                (self.flash_state, self.flash_cmd) = (0, 0);
            }
            _ => self.flash_state = 0,
        }
    }
}

impl Slot2Device for GbaCart {
    fn read(&mut self, addr: u32, width: Width) -> u32 {
        if addr >= SRAM_START {
            return read_8bit_bus(width, self.sram_read(addr));
        }
        read_16bit_bus(addr, width, |a| self.rom_half(a))
    }

    fn write(&mut self, addr: u32, value: u32, _width: Width) {
        if addr < SRAM_START {
            return; // ROM (GPIO devices are not emulated)
        }
        let (a, v) = (addr & 0xFFFF, value as u8);
        match self.save_type {
            GbaSaveType::Sram => self.save_write(a as usize, v),
            GbaSaveType::Flash64K | GbaSaveType::Flash128K => self.flash_write(a, v),
            GbaSaveType::None => {}
        }
    }

    fn tick(&mut self, _cycles: u32) -> bool {
        false
    }

    fn end_frame(&mut self) {
        if !self.dirty {
            return;
        }
        self.idle_frames += 1;
        if self.idle_frames >= FLUSH_DELAY_FRAMES
            && let Err(_e) = self.flush()
        {
            #[cfg(feature = "log")]
            log::error!("GBA save: {_e}");
        }
    }

    fn name(&self) -> &str {
        "GBA cartridge"
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

impl Drop for GbaCart {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

/// Rumble Pak: the motor moves whenever the written value changes.
#[derive(Default)]
pub struct RumblePak {
    state: u16,
    changes: u32,
    active: bool,
}

impl RumblePak {
    /// Whether the motor moved during the last frame (for host rumble).
    pub const fn is_rumbling(&self) -> bool {
        self.active
    }
}

impl Slot2Device for RumblePak {
    fn read(&mut self, addr: u32, width: Width) -> u32 {
        // A1 pulled low: FFFDh identifies the Rumble Pak.
        read_16bit_bus(addr, width, |_| 0xFFFD)
    }

    fn write(&mut self, addr: u32, value: u32, _width: Width) {
        if addr < SRAM_START && self.state != value as u16 {
            self.state = value as u16;
            self.changes += 1;
        }
    }

    fn tick(&mut self, _cycles: u32) -> bool {
        false
    }

    fn end_frame(&mut self) {
        self.active = self.changes > 0;
        self.changes = 0;
    }

    fn name(&self) -> &str {
        "Rumble Pak"
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

/// Memory Expansion Pak: 8 MiB of RAM at 0x0900_0000.
pub struct RamExpansion {
    ram: Vec<u8>,
    enable: u16,
}

impl Default for RamExpansion {
    fn default() -> Self {
        Self { ram: vec![0xFF; 0x80_0000], enable: 1 }
    }
}

impl RamExpansion {
    fn half(&self, addr: u32) -> u16 {
        let a = addr & 0x01FF_FFFF;
        match a {
            0xB2 | 0x24_0002 => 0x0000,
            0xB4 => 0x2400,
            0xB6 => 0x2424,
            0xBE | 0x1_FFFE => 0x7FFF,
            0x24_0000 => self.enable,
            0x100_0000..0x180_0000 if self.enable != 0 => {
                let i = (a & 0x7F_FFFE) as usize;
                u16::from_le_bytes([self.ram[i], self.ram[i + 1]])
            }
            // Unlisted ID-block offsets and unmapped addresses read as open bus.
            _ => 0xFFFF,
        }
    }

    fn write_half(&mut self, addr: u32, value: u16) {
        let a = addr & 0x01FF_FFFF;
        match a {
            0x24_0000 => self.enable = value & 1,
            0x100_0000..0x180_0000 if self.enable != 0 => {
                let i = (a & 0x7F_FFFE) as usize;
                self.ram[i..i + 2].copy_from_slice(&value.to_le_bytes());
            }
            _ => {}
        }
    }
}

impl Slot2Device for RamExpansion {
    fn read(&mut self, addr: u32, width: Width) -> u32 {
        if addr >= SRAM_START {
            return read_8bit_bus(width, 0xFF);
        }
        read_16bit_bus(addr, width, |a| self.half(a))
    }

    fn write(&mut self, addr: u32, value: u32, width: Width) {
        if addr >= SRAM_START {
            return;
        }
        match width {
            Width::Word => {
                self.write_half(addr & !3, value as u16);
                self.write_half((addr & !3) + 2, (value >> 16) as u16);
            }
            Width::Half => self.write_half(addr & !1, value as u16),
            Width::Byte => {
                // Byte stores reach RAM as a halfword with the byte repeated.
                let b = value as u8;
                let a = addr & 0x01FF_FFFF;
                if (0x100_0000..0x180_0000).contains(&a) && self.enable != 0 {
                    self.ram[(a & 0x7F_FFFF) as usize] = b;
                }
            }
        }
    }

    fn tick(&mut self, _cycles: u32) -> bool {
        false
    }

    fn name(&self) -> &str {
        "Memory Expansion Pak"
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

/// Guitar Grip (Guitar Hero On Tour): four fret buttons.
#[derive(Default)]
pub struct GuitarGrip {
    /// Pressed frets: green 40h, red 20h, yellow 10h, blue 08h.
    pub buttons: u8,
}

impl GuitarGrip {
    pub const GREEN: u8 = 0x40;
    pub const RED: u8 = 0x20;
    pub const YELLOW: u8 = 0x10;
    pub const BLUE: u8 = 0x08;
}

impl Slot2Device for GuitarGrip {
    fn read(&mut self, addr: u32, width: Width) -> u32 {
        if addr >= SRAM_START {
            return read_8bit_bus(width, !self.buttons);
        }
        read_16bit_bus(addr, width, |_| 0xF9FF)
    }

    fn write(&mut self, _addr: u32, _value: u32, _width: Width) {}

    fn tick(&mut self, _cycles: u32) -> bool {
        false
    }

    fn name(&self) -> &str {
        "Guitar Grip"
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gba_rom_and_flash() {
        let mut rom = vec![0; 0x200];
        rom[0xAC..0xB0].copy_from_slice(b"ABCD"); // game code
        rom[0x100..0x109].copy_from_slice(b"FLASH1M_V");
        let mut cart = GbaCart::new(rom, None);
        assert_eq!(cart.save_type(), GbaSaveType::Flash128K);
        assert_eq!(cart.read(0x0800_00AC, Width::Word), u32::from_le_bytes(*b"ABCD"));

        // program 5Ah at bank 1, offset 10h
        let w = |c: &mut GbaCart, a: u32, v: u8| c.write(SRAM_START + a, u32::from(v), Width::Byte);
        for (a, v) in [(0x5555, 0xAA), (0x2AAA, 0x55), (0x5555, 0xB0), (0, 1)] {
            w(&mut cart, a, v);
        }
        for (a, v) in [(0x5555, 0xAA), (0x2AAA, 0x55), (0x5555, 0xA0), (0x10, 0x5A)] {
            w(&mut cart, a, v);
        }
        assert_eq!(cart.save_data()[0x1_0010], 0x5A);
        assert_eq!(cart.read(SRAM_START + 0x10, Width::Byte), 0x5A);
    }

    #[test]
    fn ram_expansion_needs_enable() {
        let mut pak = RamExpansion::default();
        pak.write(0x0900_0000, 0x1234, Width::Half);
        assert_eq!(pak.read(0x0900_0000, Width::Half), 0x1234);
        pak.write(0x0824_0000, 0, Width::Half);
        assert_eq!(pak.read(0x0900_0000, Width::Half), 0xFFFF);
    }

    #[test]
    fn rumble_and_guitar_ids() {
        assert_eq!(RumblePak::default().read(0x0800_0000, Width::Half), 0xFFFD);
        let mut grip = GuitarGrip { buttons: GuitarGrip::GREEN };
        assert_eq!(grip.read(0x0800_0000, Width::Half), 0xF9FF);
        assert_eq!(grip.read(SRAM_START, Width::Byte), 0xBF);
    }
}
