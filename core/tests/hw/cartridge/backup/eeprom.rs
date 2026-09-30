//! Spec tests for `core/src/hw/cartridge/backup/eeprom.rs` — the three SPI
//! EEPROM variants (0.5 KiB / 8–64 KiB / 128 KiB).
//!
//! GBATEK "DS Cartridge Backup": <https://problemkaputt.de/gbatek.htm#dscartridgebackup>

use super::*;
use crate::test_support::{
    md::{Doc, L, R},
    scratch_dir,
    spec::Checks,
};

fn xfer(chip: &mut dyn Backup, bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            chip.write(i + 1 < bytes.len(), b);
            chip.read()
        })
        .collect()
}

fn chip<T: EEPROMType>(name: &str, size: usize) -> EEPROM<T> {
    let path = scratch_dir("eeprom").join(format!("{name}.sav"));
    let _ = std::fs::remove_file(&path);
    EEPROM::<T>::new(path, size)
}

#[test]
fn small_eeprom_uses_opcode_bit3_as_address_bit8() {
    let mut e = chip::<EEPROMSmall>("small", 0x200);
    xfer(&mut e, &[0x06]);
    xfer(&mut e, &[0x0A, 0x10, 0x77]); // WRHI → 110h
    assert_eq!(xfer(&mut e, &[0x0B, 0x10, 0])[2], 0x77, "RDHI 110h");
    assert_eq!(xfer(&mut e, &[0x03, 0x10, 0])[2], 0xFF, "RDLO 010h is a different byte");
}

#[test]
fn large_eeprom_takes_a_24_bit_address() {
    let mut e = chip::<EEPROMLarge>("large", 0x2_0000);
    xfer(&mut e, &[0x06]);
    xfer(&mut e, &[0x02, 0x01, 0x23, 0x45, 0xAB]);
    assert_eq!(xfer(&mut e, &[0x03, 0x01, 0x23, 0x45, 0])[4], 0xAB);
    assert_eq!(xfer(&mut e, &[0x03, 0x00, 0x23, 0x45, 0])[4], 0xFF, "upper address byte matters");
}

#[test]
fn write_enable_latch_clears_after_a_write_transaction() {
    let mut e = chip::<EEPROMNormal>("wel", 0x2000);
    xfer(&mut e, &[0x06]);
    xfer(&mut e, &[0x02, 0x00, 0x10, 0x01]);
    xfer(&mut e, &[0x02, 0x00, 0x11, 0x02]);
    assert_eq!(
        xfer(&mut e, &[0x03, 0x00, 0x10, 0, 0])[3..],
        [0x01, 0xFF],
        "2nd write needed a new WREN"
    );
}

#[test]
fn report() {
    let mut doc = Doc::new(
        "hw/cartridge/backup/eeprom.md",
        "SPI EEPROM saves",
        "hw/cartridge/backup/eeprom.rs",
    );
    doc.source("scratch chips created by the test");
    doc.table(
        &[("Variant", L), ("Size", R), ("Address", L), ("Lunaris type", L), ("`sram_type`", R)],
        &[
            vec!["tiny", "0.5 KiB", "8 bits + opcode bit 3 (A8)", "`EEPROMSmall`", "1"],
            vec!["normal", "8 / 64 KiB", "16 bits", "`EEPROMNormal`", "2, 3"],
            vec!["large", "128 KiB", "24 bits", "`EEPROMLarge`", "4"],
        ],
    );
    doc.table(
        &[("Opcode", L), ("Name", L), ("Params", L), ("Notes", L)],
        &[
            vec!["06h", "WREN", "–", "set WEL (cleared automatically after WR / WRSR)"],
            vec!["04h", "WRDI", "–", "clear WEL"],
            vec![
                "05h",
                "RDSR",
                "→ status",
                "bit 1 WEL, bits 2-3 BP (write protect); tiny chips read F0h in the upper nibble",
            ],
            vec!["01h", "WRSR", "status", "sets BP bits (needs WEL)"],
            vec!["03h / 0Bh", "RD / RDHI", "addr → data…", "0Bh only on tiny chips (A8 = 1)"],
            vec!["02h / 0Ah", "WR / WRHI", "addr, data…", "0Ah only on tiny chips (A8 = 1)"],
            vec!["9Fh", "RDID", "→ FFh…", "EEPROMs have no JEDEC id"],
        ],
    );
    doc.bitfield(
        "Status register (RDSR)",
        8,
        &[(7, 4, "tiny: 1111"), (3, 2, "BP"), (1, 1, "WEL"), (0, 0, "WIP")],
    );
    doc.table(
        &[("BP", L), ("Protected range", L), ("Lunaris `WriteProtect`", L)],
        &[
            vec!["00", "none", "`None`"],
            vec!["01", "upper ¼", "`UpperQuarter`"],
            vec!["10", "upper ½", "`UpperHalf`"],
            vec!["11", "all", "`All`"],
        ],
    );
    let mut c = Checks::new();
    let mut e = chip::<EEPROMNormal>("report", 0x2000);
    c.eq(
        "fresh contents",
        "erased EEPROM reads FFh",
        0xFF,
        xfer(&mut e, &[0x03, 0x00, 0x00, 0])[3],
    );
    xfer(&mut e, &[0x06]);
    c.eq("RDSR after WREN", "bit 1 set", 0x02, xfer(&mut e, &[0x05, 0])[1]);
    xfer(&mut e, &[0x02, 0x1F, 0xF0, 1, 2, 3]);
    c.eq(
        "sequential write/read",
        "address auto-increments",
        vec![1, 2, 3],
        xfer(&mut e, &[0x03, 0x1F, 0xF0, 0, 0, 0])[3..].to_vec(),
    );
    c.eq("WEL auto-clear", "WEL = 0 after the write ended", 0x00, xfer(&mut e, &[0x05, 0])[1]);
    xfer(&mut e, &[0x06]);
    xfer(&mut e, &[0x01, 0x08]); // BP = 10b → upper half
    xfer(&mut e, &[0x06]);
    xfer(&mut e, &[0x02, 0x1F, 0xF0, 9]);
    c.eq(
        "write protect",
        "BP=10b protects the upper half",
        1,
        xfer(&mut e, &[0x03, 0x1F, 0xF0, 0])[3],
    );
    c.eq("RDSR BP bits", "bits 2-3 reflect WRSR", 0x08, xfer(&mut e, &[0x05, 0])[1] & 0x0C);
    let mut t = chip::<EEPROMSmall>("report_small", 0x200);
    c.eq("tiny RDSR", "upper nibble reads 1111b", 0xF0, xfer(&mut t, &[0x05, 0])[1] & 0xF0);
    c.eq("RDID", "EEPROM has no id → FFh", 0xFF, xfer(&mut e, &[0x9F, 0])[1]);
    c.eq(
        "sram_type sizes",
        "GAME_DB size classes",
        vec![0, 0x200, 0x2000, 0x1_0000, 0x2_0000],
        <dyn Backup>::SRAM_SIZES[..5].to_vec(),
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
