//! Test ROM selection: a real cartridge dump when available, otherwise a
//! synthetic but fully spec-conformant ROM built in memory.
//!
//! Lookup order (first hit wins):
//! 1. environment variable `LUNARIS_TEST_ROM=<path to .nds>`
//! 2. first non-empty line of `core/tests/test_rom.txt` (git-ignored, so each
//!    developer can point VS Code's ▶ Run Test at their own dump)
//! 3. [`synthetic_rom`] – always available, deterministic
//!
//! Prefer a ≤ 64 MiB title: parallel tests each clone the ROM when they boot.

use std::{path::PathBuf, sync::OnceLock};

use super::crc16;

/// A ROM image plus a human-readable description of where it came from.
pub struct TestRom {
    pub bytes: Vec<u8>,
    /// `true` when `bytes` is a real cartridge dump.
    pub real: bool,
    /// File name (real) or `"synthetic"`.
    pub name: String,
}

impl TestRom {
    /// Provenance line for reports.
    pub fn label(&self) -> String {
        if self.real {
            format!("real ROM `{}` ({} bytes)", self.name, self.bytes.len())
        } else {
            format!(
                "synthetic ROM built by `core/tests/support/rom.rs` ({} bytes) — set \
                 `LUNARIS_TEST_ROM` or `core/tests/test_rom.txt` to analyse a real dump",
                self.bytes.len()
            )
        }
    }
}

/// The ROM chosen for this test run (loaded once, shared by all tests).
pub fn test_rom() -> &'static TestRom {
    static ROM: OnceLock<TestRom> = OnceLock::new();
    ROM.get_or_init(|| {
        let path = std::env::var_os("LUNARIS_TEST_ROM").map(PathBuf::from).or_else(|| {
            let cfg = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/test_rom.txt");
            std::fs::read_to_string(cfg).ok().and_then(|s| {
                s.lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty() && !l.starts_with('#'))
                    .map(PathBuf::from)
            })
        });
        if let Some(path) = path {
            if let Ok(bytes) = std::fs::read(&path) {
                let name =
                    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                return TestRom { bytes, real: true, name };
            }
            eprintln!("[spec] could not read test ROM {}; using synthetic ROM", path.display());
        }
        TestRom { bytes: synthetic_rom(), real: false, name: "synthetic".into() }
    })
}

/// The synthetic ROM (always available, independent of the real ROM).
pub fn synthetic() -> &'static TestRom {
    static ROM: OnceLock<TestRom> = OnceLock::new();
    ROM.get_or_init(|| TestRom { bytes: synthetic_rom(), real: false, name: "synthetic".into() })
}

/// Layout of [`synthetic_rom`] (all offsets are ROM file offsets).
pub mod layout {
    pub const ARM9_OFF: usize = 0x4000;
    pub const ARM9_RAM: u32 = 0x0200_0000;
    pub const ARM7_OFF: usize = 0x8000;
    pub const ARM7_RAM: u32 = 0x0238_0000;
    pub const FNT_OFF: usize = 0x8200;
    pub const FAT_OFF: usize = 0x8400;
    pub const OVT9_OFF: usize = 0x8500;
    pub const FILES_OFF: usize = 0x8600;
    pub const BANNER_OFF: usize = 0x9000;
    pub const ROM_SIZE: usize = 0x2_0000; // 128 KiB → device capacity 0
    pub const GAME_CODE: &[u8; 4] = b"LNTS";
    /// Files in FAT order: (path, contents). Id 3 is the ARM9 overlay and
    /// deliberately has no name in the FNT (overlays are addressed by id).
    pub const FILES: [(&str, &[u8]); 4] = [
        ("hello.txt", b"Hello from the Lunaris spec ROM!\n"),
        ("data/a.bin", &[0xAA; 24]),
        ("data/b.bin", &[0xBB; 40]),
        ("<overlay 0>", &[0xE1, 0xA0, 0x00, 0x00, 0xE1, 0x2F, 0xFF, 0x1E]),
    ];
}

/// Hand-assembled ARM9 program for the synthetic ROM: powers the 2D engine
/// A, selects VRAM display mode on bank A and fills it with a gradient, so a
/// headless boot of the synthetic ROM still produces a recognisable frame.
///
/// ```text
/// 00  mov  r0, #0x04000000        E3A00404
/// 04  orr  r0, r0, #0x304         E3800FC1   ; r0 = POWCNT1
/// 08  mov  r1, #0x03              E3A01003
/// 0C  orr  r1, r1, #0x8000        E3811C80   ; LCD on | 2D-A on | A=top
/// 10  str  r1, [r0]               E5801000
/// 14  mov  r0, #0x04000000        E3A00404   ; r0 = DISPCNT (A)
/// 18  mov  r1, #0x20000           E3A01802   ; display mode 2 (VRAM), bank A
/// 1C  str  r1, [r0]               E5801000
/// 20  mov  r0, #0x04000000        E3A00404
/// 24  orr  r0, r0, #0x240         E3800F90   ; r0 = VRAMCNT_A
/// 28  mov  r1, #0x80              E3A01080   ; enable, MST=0 (LCDC)
/// 2C  strb r1, [r0]               E5C01000
/// 30  mov  r0, #0x06800000        E3A00668   ; LCDC bank A
/// 34  mov  r2, #0                 E3A02000
/// 38  strh r2, [r0], #2           E0C020B2   ; loop: *r0++ = i
/// 3C  add  r2, r2, #1             E2822001
/// 40  cmp  r2, #0xC000            E3520CC0   ; 256*192 pixels
/// 44  bne  loop                   1AFFFFFB
/// 48  b    .                      EAFFFFFE
/// ```
pub const ARM9_PROGRAM: [u32; 19] = [
    0xE3A00404, 0xE3800FC1, 0xE3A01003, 0xE3811C80, 0xE5801000, 0xE3A00404, 0xE3A01802, 0xE5801000,
    0xE3A00404, 0xE3800F90, 0xE3A01080, 0xE5C01000, 0xE3A00668, 0xE3A02000, 0xE0C020B2, 0xE2822001,
    0xE3520CC0, 0x1AFFFFFB, 0xEAFFFFFE,
];

/// ARM7 program: `b .`
pub const ARM7_PROGRAM: [u32; 1] = [0xEAFFFFFE];

/// Builds a 128 KiB ROM with every structure GBATEK describes: header with
/// valid CRCs, ARM9/ARM7 binaries, FNT (root + one sub-directory), FAT, an
/// ARM9 overlay table and a banner with a 32×32 icon.
pub fn synthetic_rom() -> Vec<u8> {
    use layout::*;
    let mut rom = vec![0u8; ROM_SIZE];
    let put32 =
        |rom: &mut Vec<u8>, off: usize, v: u32| rom[off..off + 4].copy_from_slice(&v.to_le_bytes());
    let put16 =
        |rom: &mut Vec<u8>, off: usize, v: u16| rom[off..off + 2].copy_from_slice(&v.to_le_bytes());

    // --- binaries ---------------------------------------------------------
    for (i, w) in ARM9_PROGRAM.iter().enumerate() {
        put32(&mut rom, ARM9_OFF + i * 4, *w);
    }
    for (i, w) in ARM7_PROGRAM.iter().enumerate() {
        put32(&mut rom, ARM7_OFF + i * 4, *w);
    }

    // --- FNT ---------------------------------------------------------------
    // Main table: 2 directories × 8 bytes.
    let root_sub = b"\x09hello.txt\x84data\x01\xF0\x00".to_vec();
    let data_sub = b"\x05a.bin\x05b.bin\x00".to_vec();
    let main_len = 16u32;
    put32(&mut rom, FNT_OFF, main_len); // root: sub-table offset
    put16(&mut rom, FNT_OFF + 4, 0); // first file id
    put16(&mut rom, FNT_OFF + 6, 2); // total number of directories
    put32(&mut rom, FNT_OFF + 8, main_len + root_sub.len() as u32);
    put16(&mut rom, FNT_OFF + 12, 1); // first file id in "data"
    put16(&mut rom, FNT_OFF + 14, 0xF000); // parent = root
    let mut off = FNT_OFF + 16;
    rom[off..off + root_sub.len()].copy_from_slice(&root_sub);
    off += root_sub.len();
    rom[off..off + data_sub.len()].copy_from_slice(&data_sub);
    let fnt_size = (off + data_sub.len() - FNT_OFF) as u32;

    // --- FAT + file data ---------------------------------------------------
    let mut data_off = FILES_OFF;
    for (i, (_, contents)) in FILES.iter().enumerate() {
        rom[data_off..data_off + contents.len()].copy_from_slice(contents);
        put32(&mut rom, FAT_OFF + i * 8, data_off as u32);
        put32(&mut rom, FAT_OFF + i * 8 + 4, (data_off + contents.len()) as u32);
        data_off = (data_off + contents.len() + 0x1FF) & !0x1FF;
    }

    // --- ARM9 overlay table (one 32-byte entry) ----------------------------
    let ovt = [0u32, 0x0210_0000, 8, 0, 0, 0, 3, 0];
    for (i, w) in ovt.iter().enumerate() {
        put32(&mut rom, OVT9_OFF + i * 4, *w);
    }

    // --- banner (version 1, 840h bytes) ------------------------------------
    put16(&mut rom, BANNER_OFF, 0x0001);
    // Icon: 4bpp 32×32 = 4×4 tiles of 8×8; draw a diagonal gradient + border.
    for ty in 0..4 {
        for tx in 0..4 {
            for y in 0..8 {
                for x in 0..8 {
                    let (px, py) = (tx * 8 + x, ty * 8 + y);
                    let border = px == 0 || py == 0 || px == 31 || py == 31;
                    let c: u8 = if border { 15 } else { (((px + py) / 4) % 14 + 1) as u8 };
                    let tile = ty * 4 + tx;
                    let byte = BANNER_OFF + 0x20 + tile * 32 + y * 4 + x / 2;
                    rom[byte] |= if x % 2 == 0 { c } else { c << 4 };
                }
            }
        }
    }
    for i in 0..16u16 {
        // Palette: colour 0 transparent, 1..14 blue→yellow ramp, 15 white.
        let c = match i {
            0 => 0,
            15 => 0x7FFF,
            _ => {
                let t = (i - 1) * 31 / 13;
                t | (t << 5) | ((31 - t) << 10)
            }
        };
        put16(&mut rom, BANNER_OFF + 0x220 + i as usize * 2, c);
    }
    for lang in 0..6 {
        let title = "Lunaris Spec ROM\nSynthetic\nlunaris";
        for (i, u) in title.encode_utf16().enumerate() {
            put16(&mut rom, BANNER_OFF + 0x240 + lang * 0x100 + i * 2, u);
        }
    }
    let banner_crc = crc16(0xFFFF, &rom[BANNER_OFF + 0x20..BANNER_OFF + 0x840]);
    put16(&mut rom, BANNER_OFF + 2, banner_crc);

    // --- header -------------------------------------------------------------
    rom[0x000..0x00C].copy_from_slice(b"LUNARIS SPEC");
    rom[0x00C..0x010].copy_from_slice(GAME_CODE);
    rom[0x010..0x012].copy_from_slice(b"01");
    put32(&mut rom, 0x020, ARM9_OFF as u32);
    put32(&mut rom, 0x024, ARM9_RAM);
    put32(&mut rom, 0x028, ARM9_RAM);
    put32(&mut rom, 0x02C, (ARM9_PROGRAM.len() * 4) as u32);
    put32(&mut rom, 0x030, ARM7_OFF as u32);
    put32(&mut rom, 0x034, ARM7_RAM);
    put32(&mut rom, 0x038, ARM7_RAM);
    put32(&mut rom, 0x03C, (ARM7_PROGRAM.len() * 4) as u32);
    put32(&mut rom, 0x040, FNT_OFF as u32);
    put32(&mut rom, 0x044, fnt_size);
    put32(&mut rom, 0x048, FAT_OFF as u32);
    put32(&mut rom, 0x04C, (FILES.len() * 8) as u32);
    put32(&mut rom, 0x050, OVT9_OFF as u32);
    put32(&mut rom, 0x054, 32);
    put32(&mut rom, 0x060, 0x0058_6000); // typical port settings
    put32(&mut rom, 0x064, 0x0018_08F8);
    put32(&mut rom, 0x068, BANNER_OFF as u32);
    put16(&mut rom, 0x06E, 0x051E); // secure area delay
    put32(&mut rom, 0x080, data_off as u32); // used ROM size
    put32(&mut rom, 0x084, 0x4000); // header size
    // Nintendo logo intentionally left zero (copyrighted); its CRC is still
    // stored so the "logo CRC matches" invariant holds for the synthetic ROM.
    let logo_crc = crc16(0xFFFF, &rom[0x0C0..0x15C]);
    put16(&mut rom, 0x15C, logo_crc);
    let secure_crc = crc16(0xFFFF, &rom[0x4000..0x8000]);
    put16(&mut rom, 0x06C, secure_crc);
    let header_crc = crc16(0xFFFF, &rom[0x000..0x15E]);
    put16(&mut rom, 0x15E, header_crc);
    rom
}
