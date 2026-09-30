//! Spec tests for `core/src/hw/cartridge/backup.rs` — save-chip detection
//! from the game database, IR-cartridge routing and the size table.
//!
//! GBATEK "DS Cartridge Backup": <https://problemkaputt.de/gbatek.htm#dscartridgebackup>
//! GBATEK "DS Cartridge Infrared (IR)": <https://problemkaputt.de/gbatek.htm#dscartridgeinfrared>

use super::*;
use crate::test_support::{
    md::{Doc, L, R, size},
    rom, scratch_dir,
    spec::Checks,
};

/// Header of the synthetic ROM with its game code replaced.
fn header_with_code(code: &[u8; 4]) -> super::super::header::Header {
    let mut r = rom::synthetic().bytes[..0x200].to_vec();
    r[0x0C..0x10].copy_from_slice(code);
    super::super::header::Header::new(&r)
}

fn detect(code: &[u8; 4], name: &str) -> Box<dyn Backup> {
    let path = scratch_dir("backup_detect").join(format!("{name}.sav"));
    let _ = std::fs::remove_file(&path);
    <dyn Backup>::detect_type(&header_with_code(code), path)
}

/// The IR MCU answers command 08h with AAh; a plain flash chip does not.
fn is_ir(chip: &mut dyn Backup) -> bool {
    chip.write(true, 0x08);
    chip.write(false, 0x00);
    chip.read() == 0xAA
}

#[test]
fn unknown_game_falls_back_to_512k_flash() {
    let chip = detect(b"ZZZZ", "unknown");
    assert_eq!(chip.save_bytes().map(<[u8]>::len), Some(0x8_0000));
}

#[test]
fn report() {
    let mut doc = Doc::new(
        "hw/cartridge/backup.md",
        "Save chip detection (game database)",
        "hw/cartridge/backup.rs",
    );
    doc.source("built-in `GAME_DB` + synthetic headers with chosen game codes");
    doc.p("Cartridges do not describe their save chip, so Lunaris looks the header's game code up in a database (`GAME_DB`, generated from a DeSmuME-style list) and constructs the matching SPI device:");
    doc.code(
        "text",
        "header.game_code ──► GAME_DB lookup ──► sram_type ──┬─ 1      → EEPROM<EEPROMSmall>  (0.5 KiB)\n\
         \x20       │ (miss: guess from existing .sav size,    ├─ 2..3   → EEPROM<EEPROMNormal> (8/64 KiB)\n\
         \x20       │  else 6 = 512 KiB flash)                 ├─ 4      → EEPROM<EEPROMLarge>  (128 KiB)\n\
         \x20       ▼                                          ├─ 5..7   → Flash (256 K..1 M), wrapped in\n\
         game code starts with 'I' ──────────────────────────┤           IrBackup for 'I' codes (HG/SS, B/W…)\n\
         \x20                                                  ├─ 8..9   → NAND: NoBackup (not supported)\n\
         \x20                                                  └─ 0      → NoBackup",
    );
    let mut hist = [0usize; 10];
    for g in <dyn Backup>::GAME_DB.iter() {
        hist[g.sram_type.min(9)] += 1;
    }
    let rows: Vec<Vec<String>> = (0..10)
        .map(|t| vec![t.to_string(), size(<dyn Backup>::SRAM_SIZES[t] as u64), hist[t].to_string()])
        .collect();
    doc.h2(&format!("GAME_DB: {} entries by `sram_type`", <dyn Backup>::GAME_DB.len()));
    doc.table(&[("sram_type", R), ("Size", R), ("Games", R)], &rows);

    let mut c = Checks::new();
    let real = rom::test_rom();
    let code: [u8; 4] = real.bytes[0x0C..0x10].try_into().unwrap();
    let entry = <dyn Backup>::GAME_DB.iter().find(|g| g.game_code == u32::from_le_bytes(code));
    let chip = detect(&code, "real");
    doc.h2("Detection for the test ROM");
    doc.table(
        &[("Game code", L), ("DB entry", L), ("Chip size", R)],
        &[vec![
            format!("`{}`", String::from_utf8_lossy(&code)),
            entry.map(|e| format!("sram_type {}", e.sram_type)).unwrap_or("not in DB".into()),
            chip.save_bytes().map(|b| size(b.len() as u64)).unwrap_or("none".into()),
        ]],
    );
    if let Some(e) = entry.filter(|e| (1..=7).contains(&e.sram_type)) {
        c.eq(
            "DB size",
            "detected chip size = SRAM_SIZES[sram_type]",
            Some(<dyn Backup>::SRAM_SIZES[e.sram_type]),
            chip.save_bytes().map(<[u8]>::len),
        );
    }
    let unknown = detect(b"ZZZZ", "unknown_report");
    c.eq(
        "unknown code",
        "fallback = 512 KiB flash",
        Some(0x8_0000),
        unknown.save_bytes().map(<[u8]>::len),
    );
    let ir_code = <dyn Backup>::GAME_DB
        .iter()
        .find(|g| (g.game_code & 0xFF) as u8 == b'I' && (5..=7).contains(&g.sram_type));
    if let Some(g) = ir_code {
        let mut ir = detect(&g.game_code.to_le_bytes(), "ir");
        c.eq(
            &format!("IR routing ({})", String::from_utf8_lossy(&g.game_code.to_le_bytes())),
            "'I' game codes with flash saves go through the IR MCU (status 08h → AAh)",
            true,
            is_ir(ir.as_mut()),
        );
    }
    let flash_code = <dyn Backup>::GAME_DB
        .iter()
        .find(|g| (g.game_code & 0xFF) as u8 != b'I' && (5..=7).contains(&g.sram_type));
    if let Some(g) = flash_code {
        let mut f = detect(&g.game_code.to_le_bytes(), "plainflash");
        c.eq("plain flash", "non-'I' flash carts are not wrapped", false, is_ir(f.as_mut()));
    }
    c.eq(
        "size table",
        "SRAM_SIZES per GBATEK chip classes",
        vec![0, 0x200, 0x2000, 0x1_0000, 0x2_0000, 0x4_0000, 0x8_0000, 0x10_0000],
        <dyn Backup>::SRAM_SIZES[..8].to_vec(),
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
