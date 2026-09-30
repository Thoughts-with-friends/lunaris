//! Spec tests for `core/src/hw/spi.rs` — the ARM7 SPI bus and the firmware
//! flash behind it (user settings, touch calibration).
//!
//! GBATEK:
//! - "DS Serial Peripheral Interface Bus (SPI)": <https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi>
//! - "DS Firmware Header": <https://problemkaputt.de/gbatek.htm#dsfirmwareheader>
//! - "DS Firmware User Settings": <https://problemkaputt.de/gbatek.htm#dsfirmwareusersettings>

use crate::test_support::{
    boot::io_machine,
    crc16, le16,
    md::{Doc, L, R, hx},
    spec::Checks,
};

pub(crate) const SPICNT: u32 = 0x0400_01C0;
pub(crate) const SPIDATA: u32 = 0x0400_01C2;

/// One SPI transaction to `device` (0 power, 1 firmware, 2 touchscreen):
/// HOLD stays set for every byte but the last.
pub(crate) fn spi(hw: &mut crate::hw::HW, device: u16, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        let hold = if i + 1 < bytes.len() { 0x0800 } else { 0 };
        hw.arm7_write::<u16>(SPICNT, 0x8000 | hold | device << 8);
        hw.arm7_write::<u8>(SPIDATA, b);
        out.push(hw.arm7_read::<u8>(SPIDATA));
    }
    hw.arm7_write::<u16>(SPICNT, 0);
    out
}

/// Reads `len` firmware bytes at `addr` with the READ (03h) command.
pub(crate) fn firmware_read(hw: &mut crate::hw::HW, addr: u32, len: usize) -> Vec<u8> {
    let mut cmd = vec![0x03, (addr >> 16) as u8, (addr >> 8) as u8, addr as u8];
    cmd.extend(std::iter::repeat_n(0, len));
    spi(hw, 1, &cmd)[4..].to_vec()
}

#[test]
fn firmware_read_matches_the_free_firmware_image() {
    let mut nds = io_machine("spi_fw");
    let got = firmware_read(nds.hw_mut(), 0, 0x40);
    assert_eq!(got, &free_bios::firmware::FIRMWARE_DS[..0x40]);
}

#[test]
fn touch_calibration_is_patched_to_identity() {
    let mut nds = io_machine("spi_calib");
    let us = firmware_read(nds.hw_mut(), 0x3FE00, 0x74);
    assert_eq!((le16(&us, 0x58), le16(&us, 0x5A), us[0x5C], us[0x5D]), (0, 0, 0, 0));
    assert_eq!(
        (le16(&us, 0x5E), le16(&us, 0x60), us[0x62], us[0x63]),
        (255 << 4, 191 << 4, 255, 191)
    );
    assert_eq!(le16(&us, 0x72), crc16(0xFFFF, &us[..0x70]));
}

#[test]
fn report() {
    let mut nds = io_machine("spi_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/spi.md", "SPI bus & firmware flash", "hw/spi.rs");
    doc.source("`free_bios::firmware::FIRMWARE_DS` (256 KiB), after Lunaris' calibration patch");
    doc.bitfield(
        "40001C0h SPICNT (ARM7)",
        16,
        &[
            (15, 15, "EN"),
            (14, 14, "IRQ"),
            (11, 11, "HOLD"),
            (10, 10, "16BIT"),
            (9, 8, "DEVICE"),
            (7, 7, "BUSY"),
            (1, 0, "BAUD"),
        ],
    );
    doc.table(
        &[("DEVICE", R), ("Chip", L), ("Lunaris", L)],
        &[
            vec!["0", "power management", "ignored (reads 0)"],
            vec!["1", "firmware flash (256 KiB)", "`SPI::firmware: Flash`"],
            vec!["2", "touchscreen TSC2046", "`SPI::tsc: TSC`"],
            vec!["3", "reserved", "panics"],
        ],
    );
    let head = firmware_read(hw, 0, 0x40);
    doc.h2("Firmware header (000h–03Fh)");
    let fields: &[(usize, usize, &str, &str)] = &[
        (0x00, 2, "part3_romaddr", "ARM9 GUI code address / 8"),
        (0x02, 2, "part4_romaddr", "ARM7 Wi-Fi code address / 8"),
        (0x04, 2, "part34_crc", "CRC16 of parts 3+4"),
        (0x06, 2, "part12_crc", "CRC16 of parts 1+2"),
        (0x08, 4, "fw_identifier", "\"MAC\" + version"),
        (0x0C, 2, "part1_romaddr", "ARM9 boot code address / 4"),
        (0x0E, 2, "part1_ramaddr", "02800000h - address / 4"),
        (0x10, 2, "part2_romaddr", "ARM7 boot code address / 8"),
        (0x12, 2, "part2_ramaddr", "03810000h - address / 4"),
        (0x14, 2, "shift_amounts", "part1/2 relocation shifts"),
        (0x16, 2, "part5_romaddr", "data/gfx address / 8"),
        (0x18, 5, "timestamp", "build date (BCD)"),
        (0x1D, 1, "console_type", "FFh DS, 20h DS-lite, 57h DSi…"),
        (0x20, 2, "user_settings_offset", "address / 8 (normally 7FC0h → 3FE00h)"),
        (0x2A, 2, "wifi_cfg_crc", "CRC16 of Wi-Fi config (02Ch..)"),
        (0x2C, 2, "wifi_cfg_length", "Wi-Fi config length"),
    ];
    let rows: Vec<Vec<String>> = fields
        .iter()
        .map(|&(off, len, name, desc)| {
            let v = head[off..off + len].iter().rev().fold(0u64, |a, &b| a << 8 | b as u64);
            vec![
                format!("`{name}`"),
                desc.into(),
                len.to_string(),
                hx(off as u64, 3),
                hx(v, len * 2),
            ]
        })
        .collect();
    doc.table(&[("Field", L), ("Description", L), ("Size", R), ("Offset", R), ("Value", L)], &rows);
    doc.hexdump(0, &head);

    let us = firmware_read(hw, 0x3FE00, 0x100);
    doc.h2("User settings (3FE00h, first copy)");
    let nick: String = String::from_utf16_lossy(
        &(0..us[0x1A].min(10) as usize).map(|i| le16(&us, 0x06 + i * 2)).collect::<Vec<_>>(),
    );
    let urows = vec![
        vec![
            "`version`".into(),
            "always 5".into(),
            "2".into(),
            "0x000".into(),
            hx(le16(&us, 0) as u64, 4),
        ],
        vec![
            "`favorite_color`".into(),
            "0..15".into(),
            "1".into(),
            "0x002".into(),
            us[2].to_string(),
        ],
        vec![
            "`birthday`".into(),
            "month / day".into(),
            "2".into(),
            "0x003".into(),
            format!("{}/{}", us[3], us[4]),
        ],
        vec![
            "`nickname`".into(),
            "UTF-16, 10 chars".into(),
            "20".into(),
            "0x006".into(),
            format!("\"{nick}\""),
        ],
        vec!["`nickname_len`".into(), "".into(), "2".into(), "0x01A".into(), us[0x1A].to_string()],
        vec![
            "`adc_x1, adc_y1`".into(),
            "touch ADC at point 1".into(),
            "4".into(),
            "0x058".into(),
            format!("{}, {}", le16(&us, 0x58), le16(&us, 0x5A)),
        ],
        vec![
            "`scr_x1, scr_y1`".into(),
            "screen pixel of point 1".into(),
            "2".into(),
            "0x05C".into(),
            format!("{}, {}", us[0x5C], us[0x5D]),
        ],
        vec![
            "`adc_x2, adc_y2`".into(),
            "touch ADC at point 2".into(),
            "4".into(),
            "0x05E".into(),
            format!("{}, {}", le16(&us, 0x5E), le16(&us, 0x60)),
        ],
        vec![
            "`scr_x2, scr_y2`".into(),
            "screen pixel of point 2".into(),
            "2".into(),
            "0x062".into(),
            format!("{}, {}", us[0x62], us[0x63]),
        ],
        vec![
            "`language_flags`".into(),
            "bits 0-2 language".into(),
            "2".into(),
            "0x064".into(),
            hx(le16(&us, 0x64) as u64, 4),
        ],
        vec![
            "`update_counter`".into(),
            "newer copy wins".into(),
            "2".into(),
            "0x070".into(),
            le16(&us, 0x70).to_string(),
        ],
        vec![
            "`crc16`".into(),
            "CRC16(FFFFh, 000h..06Fh)".into(),
            "2".into(),
            "0x072".into(),
            hx(le16(&us, 0x72) as u64, 4),
        ],
    ];
    doc.table(
        &[("Field", L), ("Description", L), ("Size", R), ("Offset", R), ("Value", L)],
        &urows,
    );
    doc.note("`SPI::init_firmware` rewrites the calibration to the identity mapping (ADC = pixel × 16) and fixes the CRC, so the touchscreen needs no per-user calibration.");
    let mut c = Checks::new();
    c.ok(
        "firmware READ",
        "03h aa aa aa → bytes of the image",
        head == free_bios::firmware::FIRMWARE_DS[..0x40],
        "first 40h bytes compared",
    );
    c.hex(
        "user-settings CRC",
        "072h = CRC16(000h..06Fh)",
        crc16(0xFFFF, &us[..0x70]),
        le16(&us, 0x72),
    );
    c.eq(
        "calibration point 2",
        "ADC (FF0h, BF0h) ↔ pixel (255, 191)",
        (0xFF0, 0xBF0, 255, 191),
        (le16(&us, 0x5E), le16(&us, 0x60), us[0x62], us[0x63]),
    );
    c.eq("SPICNT read-back", "EN|HOLD|device", 0x8900u16, {
        hw.arm7_write::<u16>(SPICNT, 0x8900);
        hw.arm7_read::<u16>(SPICNT)
    });
    hw.arm7_write::<u16>(SPICNT, 0);
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
