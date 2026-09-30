//! Spec tests for `core/src/hw/cartridge/backup/flash.rs` — SPI FLASH save
//! chips (also used for the console firmware).
//!
//! GBATEK "DS Cartridge Backup": <https://problemkaputt.de/gbatek.htm#dscartridgebackup>

use super::*;
use crate::test_support::{
    md::{Doc, L, R, hx},
    scratch_dir,
    spec::Checks,
};

/// One chip-select-low transaction: every byte but the last keeps HOLD set.
/// Returns the byte clocked out for each byte written.
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

fn chip(name: &str, size: usize) -> Flash {
    let path = scratch_dir("flash").join(format!("{name}.sav"));
    let _ = std::fs::remove_file(&path);
    Flash::new_backup(path, size)
}

#[test]
fn fresh_chip_reads_ffh() {
    let mut f = chip("fresh", 0x4_0000);
    assert_eq!(xfer(&mut f, &[0x03, 0, 0, 0, 0, 0])[4..], [0xFF, 0xFF]);
}

#[test]
fn page_program_needs_wren_and_only_clears_bits() {
    let mut f = chip("pp", 0x4_0000);
    xfer(&mut f, &[0x02, 0, 0x01, 0x00, 0x0F]);
    assert_eq!(xfer(&mut f, &[0x03, 0, 0x01, 0x00, 0])[4], 0xFF, "no WREN → ignored");
    xfer(&mut f, &[0x06]);
    xfer(&mut f, &[0x02, 0, 0x01, 0x00, 0x0F]);
    xfer(&mut f, &[0x06]);
    xfer(&mut f, &[0x02, 0, 0x01, 0x00, 0xF3]);
    assert_eq!(xfer(&mut f, &[0x03, 0, 0x01, 0x00, 0])[4], 0x03, "PP ANDs: 0Fh & F3h");
}

#[test]
fn page_write_replaces_and_wraps_within_256_byte_page() {
    let mut f = chip("pw", 0x4_0000);
    xfer(&mut f, &[0x06]);
    xfer(&mut f, &[0x0A, 0, 0x02, 0xFF, 0x11, 0x22]);
    let r = xfer(&mut f, &[0x03, 0, 0x02, 0x00, 0]);
    assert_eq!(r[4], 0x22, "second byte wrapped to page offset 0");
}

#[test]
fn report() {
    let mut f = chip("report", 0x4_0000);
    let mut doc = Doc::new(
        "hw/cartridge/backup/flash.md",
        "SPI FLASH (saves & firmware)",
        "hw/cartridge/backup/flash.rs",
    );
    doc.source("scratch 256 KiB chip created by the test");
    doc.p("Every transaction starts with chip select going low: the first byte is the instruction, followed by a 24-bit big-endian address and data. AUXSPICNT bit 6 (HOLD) keeps CS low between bytes; releasing it ends the instruction (`Backup::write(hold=false)` → `Mode::ReadInstr`).");
    doc.code(
        "text",
        "CS  ‾‾\\____________________________________________/‾‾\n\
         MOSI    [ 03h ][ A23-16 ][ A15-8 ][ A7-0 ][ xx ][ xx ] …\n\
         MISO                                     [ D0 ][ D1 ] …   (READ, address auto-increments)",
    );
    doc.table(
        &[("Opcode", L), ("Name", L), ("Params", L), ("Lunaris `Instr`", L), ("Effect", L)],
        &[
            vec!["06h", "WREN", "–", "`WREN`", "set write-enable latch"],
            vec!["04h", "WRDI", "–", "`WRDI`", "clear write-enable latch"],
            vec!["05h", "RDSR", "→ status", "`RDSR`", "bit 1 = WEL"],
            vec!["9Fh", "RDID", "→ 3 bytes", "`RDID`", "JEDEC id (Lunaris: 00h)"],
            vec!["03h", "READ", "addr24 → data…", "`READ`", "sequential read"],
            vec![
                "0Bh",
                "FAST READ",
                "addr24, dummy → data…",
                "`FastReadAddr`/`FastRead`",
                "read after 1 dummy byte",
            ],
            vec![
                "0Ah",
                "PW",
                "addr24, data…",
                "`PW`",
                "page write (erase+program), wraps in 256 B page",
            ],
            vec!["02h", "PP", "addr24, data…", "`PP`", "page program: `mem &= data`"],
            vec!["DBh", "PE", "addr24", "`PE`", "page erase (256 B → FFh)"],
            vec!["D8h", "SE", "addr24", "`SE`", "sector erase (64 KiB → FFh)"],
        ],
    );
    let mut c = Checks::new();
    c.eq("RDSR idle", "WEL = 0", 0x00, xfer(&mut f, &[0x05, 0])[1]);
    xfer(&mut f, &[0x06]);
    c.eq("RDSR after WREN", "WEL = 1 → 02h", 0x02, xfer(&mut f, &[0x05, 0])[1]);
    xfer(&mut f, &[0x04]);
    c.eq("RDSR after WRDI", "WEL = 0", 0x00, xfer(&mut f, &[0x05, 0])[1]);
    xfer(&mut f, &[0x06]);
    xfer(&mut f, &[0x0A, 0x00, 0x10, 0x00, 0xDE, 0xAD, 0xBE, 0xEF]);
    c.eq(
        "PW + READ",
        "written bytes read back",
        vec![0xDE, 0xAD, 0xBE, 0xEF],
        xfer(&mut f, &[0x03, 0x00, 0x10, 0x00, 0, 0, 0, 0])[4..].to_vec(),
    );
    c.known(
        "FAST READ",
        "0Bh addr24 + ONE dummy byte, then data",
        vec![0xDE, 0xAD],
        xfer(&mut f, &[0x0B, 0x00, 0x10, 0x00, 0, 0, 0])[5..].to_vec(),
        "`FastReadAddr(0, _)` consumes an extra byte before entering the dummy phase, so data starts one byte late",
    );
    xfer(&mut f, &[0x06]);
    xfer(&mut f, &[0xDB, 0x00, 0x10, 0x80]);
    c.known(
        "PE (4-byte transaction)",
        "DBh addr24, CS high → page (256 B) = FFh",
        0xFF,
        xfer(&mut f, &[0x03, 0x00, 0x10, 0x00, 0])[4],
        "the erase runs in `handle_instr(PE(0, _))`, i.e. only when a 5th byte is clocked; CS release after the address resets the mode without erasing",
    );
    xfer(&mut f, &[0x06]);
    xfer(&mut f, &[0x0A, 0x01, 0x00, 0x00, 0x55]);
    xfer(&mut f, &[0x06]);
    xfer(&mut f, &[0xD8, 0x01, 0x23, 0x45]);
    c.known(
        "SE (4-byte transaction)",
        "D8h addr24, CS high → sector (64 KiB) = FFh",
        0xFF,
        xfer(&mut f, &[0x03, 0x01, 0x00, 0x00, 0])[4],
        "same as PE: the erase needs an extra clocked byte",
    );
    xfer(&mut f, &[0x06]);
    xfer(&mut f, &[0xD8, 0x01, 0x23, 0x45, 0x00]);
    c.eq(
        "SE (5-byte transaction)",
        "with one trailing byte Lunaris does erase",
        0xFF,
        xfer(&mut f, &[0x03, 0x01, 0x00, 0x00, 0])[4],
    );
    c.eq(
        "address mirror",
        "reads past the end wrap (SaveMem::read)",
        xfer(&mut f, &[0x03, 0x00, 0x00, 0x00, 0])[4],
        xfer(&mut f, &[0x03, 0x04, 0x00, 0x00, 0])[4],
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.table(
        &[("Size", R), ("Typical chip", L)],
        &[
            vec![hx(0x4_0000, 5), "ST M45PE20 (256 KiB)".into()],
            vec![hx(0x8_0000, 5), "512 KiB (Pokémon HG/SS, B/W)".into()],
            vec![hx(0x10_0000, 6), "1 MiB".into()],
        ],
    );
    doc.save();
    c.finish();
}
