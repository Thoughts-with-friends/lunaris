//! Spec tests for `core/src/hw/cartridge/header.rs` — the 200h-byte NDS
//! cartridge header.
//!
//! GBATEK "DS Cartridge Header": <https://problemkaputt.de/gbatek.htm#dscartridgeheader>

use super::*;
use crate::test_support::{
    ascii, crc16, le16, le32,
    md::{Doc, L, R, hx, size},
    rom::{self, TestRom},
    spec::Checks,
};

/// `(offset, size, rust field, GBATEK description)` for every header field
/// Lunaris parses, in ROM order.
const FIELDS: &[(usize, usize, &str, &str)] = &[
    (0x000, 12, "game_title", "Game title, uppercase ASCII, zero-padded"),
    (0x00C, 4, "game_code", "Game code (ASCII, e.g. `A2DJ`); 0 = homebrew"),
    (0x010, 2, "maker_code", "Maker code (ASCII, `01` = Nintendo)"),
    (0x012, 1, "unit_code", "Unit code: 00h NDS, 02h NDS+DSi, 03h DSi"),
    (0x013, 1, "encryption_seed", "Encryption seed select (0..7) for KEY2"),
    (0x014, 1, "device_capacity", "Chip size = 128 KiB << n"),
    (0x015, 7, "reserved0", "Reserved (zero)"),
    (0x01C, 1, "reserved1", "Reserved (DSi flags)"),
    (0x01D, 1, "region", "NDS region: 00h normal, 40h Korea, 80h China"),
    (0x01E, 1, "rom_version", "ROM version (usually 0)"),
    (0x01F, 1, "autostart", "Bit 2: skip \"Press Button\" after health screen"),
    (0x020, 4, "arm9_rom_offset", "ARM9 binary ROM offset (4000h-aligned)"),
    (0x024, 4, "arm9_entry_addr", "ARM9 entry address (2000000h..23BFE00h)"),
    (0x028, 4, "arm9_ram_addr", "ARM9 load address in RAM"),
    (0x02C, 4, "arm9_size", "ARM9 binary size (max 3BFE00h)"),
    (0x030, 4, "arm7_rom_offset", "ARM7 binary ROM offset (200h-aligned)"),
    (0x034, 4, "arm7_entry_addr", "ARM7 entry address"),
    (0x038, 4, "arm7_ram_addr", "ARM7 load address in RAM"),
    (0x03C, 4, "arm7_size", "ARM7 binary size (max 3BFE00h / FE00h)"),
    (0x040, 4, "fnt_offset", "File Name Table offset"),
    (0x044, 4, "fnt_size", "File Name Table size"),
    (0x048, 4, "fat_offset", "File Allocation Table offset"),
    (0x04C, 4, "fat_size", "File Allocation Table size (8 bytes per file)"),
    (0x050, 4, "arm9_overlay_offset", "ARM9 overlay table offset"),
    (0x054, 4, "arm9_overlay_size", "ARM9 overlay table size (32 bytes per overlay)"),
    (0x058, 4, "arm7_overlay_offset", "ARM7 overlay table offset"),
    (0x05C, 4, "arm7_overlay_size", "ARM7 overlay table size"),
    (0x060, 4, "port_settings_normal", "ROMCTRL value for normal commands"),
    (0x064, 4, "port_settings_key1", "ROMCTRL value for KEY1 commands"),
    (0x068, 4, "icon_offset", "Icon/title (banner) offset, 0 = none"),
    (0x06C, 2, "secure_area_checksum", "CRC16 of ROM 4000h..7FFFh (encrypted form)"),
    (0x06E, 2, "secure_area_delay", "Secure area delay in 131 kHz units"),
    (0x070, 4, "arm9_auto_load_list_hook_ram_addr", "ARM9 auto-load list hook"),
    (0x074, 4, "arm7_auto_load_list_hook_ram_addr", "ARM7 auto-load list hook"),
    (0x078, 8, "secure_area_disable", "Secure area disable (\"NmMdOnly\" when disabled)"),
    (0x080, 4, "used_rom_size", "Total used ROM size (excluding DSi area)"),
    (0x084, 4, "header_size", "ROM header size (4000h)"),
    (0x088, 40, "reserved2", "Reserved (DSi uses the first bytes)"),
    (0x0B0, 16, "reserved3", "Reserved (zero)"),
    (0x0C0, 156, "nintendo_logo", "Compressed Nintendo logo bitmap"),
    (0x15C, 2, "nintendo_logo_checksum", "CRC16 of 0C0h..15Bh, always CF56h"),
    (0x15E, 2, "header_checksum", "CRC16 of 000h..15Dh"),
];

/// Raw value of a field as the header parser should see it.
fn raw(rom: &[u8], off: usize, len: usize) -> u64 {
    match len {
        1 => rom[off] as u64,
        2 => le16(rom, off) as u64,
        4 => le32(rom, off) as u64,
        8 => le32(rom, off) as u64 | (le32(rom, off + 4) as u64) << 32,
        _ => 0,
    }
}

/// The same value read back from Lunaris's parsed [`Header`].
fn parsed(h: &Header, field: &str) -> Option<u64> {
    Some(match field {
        "game_code" => h.game_code as u64,
        "unit_code" => match h.unit_code {
            UnitCode::NDS => 0,
            UnitCode::Both => 2,
            UnitCode::DSi => 3,
        },
        "encryption_seed" => h.encryption_seed as u64,
        "device_capacity" => h.device_capacity as u64,
        "reserved1" => h.reserved1 as u64,
        "region" => match h.region {
            Region::Normal => 0,
            Region::Korea => 0x40,
            Region::China => 0x80,
        },
        "rom_version" => h.rom_version as u64,
        "autostart" => h.autostart as u64,
        "arm9_rom_offset" => h.arm9_rom_offset as u64,
        "arm9_entry_addr" => h.arm9_entry_addr as u64,
        "arm9_ram_addr" => h.arm9_ram_addr as u64,
        "arm9_size" => h.arm9_size as u64,
        "arm7_rom_offset" => h.arm7_rom_offset as u64,
        "arm7_entry_addr" => h.arm7_entry_addr as u64,
        "arm7_ram_addr" => h.arm7_ram_addr as u64,
        "arm7_size" => h.arm7_size as u64,
        "fnt_offset" => h.fnt_offset as u64,
        "fnt_size" => h.fnt_size as u64,
        "fat_offset" => h.fat_offset as u64,
        "fat_size" => h.fat_size as u64,
        "arm9_overlay_offset" => h.arm9_overlay_offset as u64,
        "arm9_overlay_size" => h.arm9_overlay_size as u64,
        "arm7_overlay_offset" => h.arm7_overlay_offset as u64,
        "arm7_overlay_size" => h.arm7_overlay_size as u64,
        "port_settings_normal" => h.port_settings_normal as u64,
        "port_settings_key1" => h.port_settings_key1 as u64,
        "icon_offset" => h.icon_offset as u64,
        "secure_area_checksum" => h.secure_area_checksum as u64,
        "secure_area_delay" => h.secure_area_delay as u64,
        "arm9_auto_load_list_hook_ram_addr" => h.arm9_auto_load_list_hook_ram_addr as u64,
        "arm7_auto_load_list_hook_ram_addr" => h.arm7_auto_load_list_hook_ram_addr as u64,
        "secure_area_disable" => h.secure_area_disable,
        "used_rom_size" => h.used_rom_size as u64,
        "header_size" => h.header_size as u64,
        "nintendo_logo_checksum" => h.nintendo_logo_checksum as u64,
        "header_checksum" => h.header_checksum as u64,
        _ => return None,
    })
}

fn rust_type(len: usize) -> String {
    match len {
        1 => "u8".into(),
        2 => "u16".into(),
        4 => "u32".into(),
        8 => "u64".into(),
        n => format!("[u8; {n:#X}]"),
    }
}

fn decoded(rom: &[u8], off: usize, len: usize, field: &str) -> String {
    let b = &rom[off..off + len];
    match field {
        "game_title" | "game_code" | "maker_code" => {
            format!("`{}`", ascii(b).trim_end_matches('.'))
        }
        "device_capacity" => size(0x2_0000u64 << b[0].min(20)),
        "fat_size" => format!("{} files", raw(rom, off, len) / 8),
        "arm9_overlay_size" | "arm7_overlay_size" => {
            format!("{} overlays", raw(rom, off, len) / 32)
        }
        "arm9_size" | "arm7_size" | "fnt_size" | "used_rom_size" | "header_size" => {
            size(raw(rom, off, len))
        }
        "secure_area_delay" => format!("{:.2} ms", raw(rom, off, len) as f64 / 131.072),
        "secure_area_disable" => format!("`{}`", ascii(b)),
        "autostart" => format!("skip press-button = {}", b[0] & 4 != 0),
        _ if len > 8 => {
            let nz = b.iter().filter(|&&x| x != 0).count();
            format!("{nz} non-zero bytes")
        }
        _ => String::new(),
    }
}

#[test]
fn parser_reads_every_field_at_its_gbatek_offset() {
    for rom in [rom::synthetic(), rom::test_rom()] {
        let h = Header::new(&rom.bytes);
        for &(off, len, field, _) in FIELDS {
            if let Some(v) = parsed(&h, field) {
                assert_eq!(v, raw(&rom.bytes, off, len), "{field} @ {off:#05X} ({})", rom.name);
            }
        }
        assert_eq!(&h.game_title[..], &rom.bytes[0..12]);
        assert_eq!(&h.maker_code[..], &rom.bytes[0x10..0x12]);
        assert_eq!(&h.nintendo_logo[..], &rom.bytes[0xC0..0x15C]);
    }
}

#[test]
fn fields_are_contiguous_and_cover_000h_to_160h() {
    let mut next = 0;
    for &(off, len, field, _) in FIELDS {
        assert_eq!(off, next, "gap before {field}");
        next = off + len;
    }
    assert_eq!(next, 0x160);
}

#[test]
fn crc16_matches_known_vectors() {
    // GBATEK: the Nintendo logo CRC is always CF56h; the CRC of an empty
    // buffer is the initial value.
    assert_eq!(crc16(0xFFFF, &[]), 0xFFFF);
    // CRC-16/MODBUS reference vector ("123456789" → 4B37h).
    assert_eq!(crc16(0xFFFF, b"123456789"), 0x4B37);
}

/// Checks that hold for any well-formed ROM, real or synthetic.
fn header_checks(rom: &TestRom, c: &mut Checks) {
    let b = &rom.bytes;
    let h = Header::new(b);
    c.hex(
        "header CRC16",
        "15Eh = CRC16(FFFFh, 000h..15Dh)",
        crc16(0xFFFF, &b[..0x15E]),
        h.header_checksum,
    );
    c.hex(
        "logo CRC16",
        "15Ch = CRC16(FFFFh, 0C0h..15Bh)",
        crc16(0xFFFF, &b[0xC0..0x15C]),
        h.nintendo_logo_checksum,
    );
    if rom.real {
        c.hex(
            "logo CRC constant",
            "15Ch is always CF56h on retail carts",
            0xCF56u16,
            h.nintendo_logo_checksum,
        );
    }
    c.ok(
        "ARM9 load range",
        "ARM9 RAM address in 2000000h..23BFE00h",
        (0x0200_0000..=0x023B_FE00).contains(&h.arm9_ram_addr),
        hx(h.arm9_ram_addr as u64, 8),
    );
    c.ok(
        "ARM9 entry inside binary",
        "entry ∈ [ram_addr, ram_addr + size)",
        (h.arm9_ram_addr..h.arm9_ram_addr + h.arm9_size.max(4)).contains(&h.arm9_entry_addr),
        hx(h.arm9_entry_addr as u64, 8),
    );
    c.ok("ARM9 size", "max 3BFE00h", h.arm9_size <= 0x3B_FE00, hx(h.arm9_size as u64, 6));
    let arm7_ok = (0x0200_0000..=0x023B_FE00).contains(&h.arm7_ram_addr)
        || (0x037F_8000..=0x0380_7E00).contains(&h.arm7_ram_addr);
    c.ok(
        "ARM7 load range",
        "2000000h..23BFE00h or 37F8000h..3807E00h",
        arm7_ok,
        hx(h.arm7_ram_addr as u64, 8),
    );
    c.ok(
        "ARM9 ROM offset",
        "≥ 4000h",
        h.arm9_rom_offset >= 0x4000,
        hx(h.arm9_rom_offset as u64, 6),
    );
    c.ok(
        "binaries inside ROM",
        "offset + size ≤ ROM length",
        (h.arm9_rom_offset + h.arm9_size) as usize <= b.len()
            && (h.arm7_rom_offset + h.arm7_size) as usize <= b.len(),
        format!("ROM = {}", size(b.len() as u64)),
    );
    c.ok(
        "device capacity",
        "ROM length ≤ 128 KiB << capacity",
        b.len() as u64 <= 0x2_0000u64 << h.device_capacity.min(20),
        size(0x2_0000u64 << h.device_capacity.min(20)),
    );
    c.eq("FAT entry size", "fat_size is a multiple of 8", 0, h.fat_size % 8);
    c.eq(
        "overlay entry size",
        "arm9_overlay_size is a multiple of 32",
        0,
        h.arm9_overlay_size % 32,
    );
    c.hex("header size", "084h = 4000h", 0x4000u32, h.header_size);
}

#[test]
fn synthetic_header_is_well_formed() {
    let mut c = Checks::new();
    header_checks(rom::synthetic(), &mut c);
    c.finish();
}

#[test]
fn report() {
    let rom = rom::test_rom();
    let b = &rom.bytes;
    let mut doc = Doc::new(
        "hw/cartridge/header.md",
        "Cartridge header (000h–1FFh)",
        "hw/cartridge/header.rs",
    );
    doc.source(&rom.label());
    doc.p("The first 200h bytes of every cartridge describe where everything else lives. \
         The BIOS (or Lunaris' direct boot) copies 000h–16Fh to main RAM at 27FFE00h and \
         uses the ARM9/ARM7 entries below to load and start both CPUs.")
        .note("All multi-byte values are little-endian. Offsets are 0-based ROM file offsets.");

    doc.h2("Header layout (Lunaris `Header` struct ⇄ ROM bytes)");
    let rows: Vec<Vec<String>> = FIELDS
        .iter()
        .map(|&(off, len, field, desc)| {
            let value = if len <= 8 { hx(raw(b, off, len), len * 2) } else { "…".into() };
            vec![
                format!("`{field}`"),
                desc.into(),
                rust_type(len),
                format!("{len}"),
                hx(off as u64, 3),
                value,
                decoded(b, off, len, field),
            ]
        })
        .collect();
    doc.table(
        &[
            ("Field", L),
            ("Description", L),
            ("Rust Type", R),
            ("Size", R),
            ("Offset", R),
            ("Value", L),
            ("Decoded", L),
        ],
        &rows,
    );

    doc.h2("Byte map of the header");
    doc.code(
        "text",
        "000h ┌────────────────────────────┬──────────┬─────┬──┬──┬──┬───────┬──┬──┬──┬──┐\n\
         \x20    │ game_title (12)            │ code (4) │mk(2)│uc│es│dc│ rsv(7)│r1│rg│rv│as│\n\
         020h ├──────────┬──────────┬───────┴──┬───────┴──┬──┴──┴──┴───────┴──┴──┴──┴──┤\n\
         \x20    │ARM9 rom  │ARM9 entry│ARM9 ram  │ARM9 size │  ← ARM9 binary descriptor    │\n\
         030h ├──────────┼──────────┼──────────┼──────────┤                              │\n\
         \x20    │ARM7 rom  │ARM7 entry│ARM7 ram  │ARM7 size │  ← ARM7 binary descriptor    │\n\
         040h ├──────────┼──────────┼──────────┼──────────┤                              │\n\
         \x20    │FNT off   │FNT size  │FAT off   │FAT size  │  ← file system               │\n\
         050h ├──────────┼──────────┼──────────┼──────────┤                              │\n\
         \x20    │OVT9 off  │OVT9 size │OVT7 off  │OVT7 size │  ← overlays                  │\n\
         060h ├──────────┼──────────┼──────────┼────┬─────┤                              │\n\
         \x20    │ROMCTRL   │ROMCTRL k1│icon off  │sCRC│delay│                              │\n\
         070h ├──────────┼──────────┼──────────┴────┴─────┤                              │\n\
         \x20    │ARM9 hook │ARM7 hook │ secure_area_disable │                              │\n\
         080h ├──────────┼──────────┼─────────────────────┴──────────────────────────────┤\n\
         \x20    │used size │hdr size  │ reserved2 (28h) … reserved3 (10h)                  │\n\
         0C0h ├──────────┴──────────┴────────────────────────────────────────────────────┤\n\
         \x20    │ nintendo_logo (9Ch bytes, compressed 1bpp bitmap)                       │\n\
         15Ch ├────────┬────────┬───────────────────────────────────────────────────────┤\n\
         \x20    │logo CRC│hdr CRC │ debug ROM fields / reserved (not parsed by Lunaris)    │\n\
         200h └────────┴────────┴───────────────────────────────────────────────────────┘",
    );

    doc.h2("ROM map derived from the header");
    let h = Header::new(b);
    let mut regions = vec![
        ("header", 0u64, 0x200u64),
        ("ARM9 binary", h.arm9_rom_offset as u64, h.arm9_size as u64),
        ("ARM7 binary", h.arm7_rom_offset as u64, h.arm7_size as u64),
        ("FNT (file names)", h.fnt_offset as u64, h.fnt_size as u64),
        ("FAT (file extents)", h.fat_offset as u64, h.fat_size as u64),
        ("ARM9 overlay table", h.arm9_overlay_offset as u64, h.arm9_overlay_size as u64),
        ("ARM7 overlay table", h.arm7_overlay_offset as u64, h.arm7_overlay_size as u64),
        ("banner (icon/title)", h.icon_offset as u64, if h.icon_offset != 0 { 0x840 } else { 0 }),
    ];
    regions.retain(|r| r.2 > 0);
    regions.sort_by_key(|r| r.1);
    let used = (h.used_rom_size as u64).max(regions.iter().map(|r| r.1 + r.2).max().unwrap_or(0));
    let mut art = String::new();
    art += &format!("{:19} 0{:>47}\n", "", format!("{} (used_rom_size)", hx(used, 8)));
    for (name, off, len) in &regions {
        // Position track: where inside the used ROM the region sits.
        let a = (off * 48 / used.max(1)).min(47) as usize;
        let e = (((off + len) * 48).div_ceil(used.max(1)) as usize).clamp(a + 1, 48);
        let track: String = (0..48).map(|i| if (a..e).contains(&i) { '█' } else { '·' }).collect();
        art += &format!("{:08X}..{:08X} │{track}│ {name} ({})\n", off, off + len, size(*len));
    }
    doc.code("text", &art);

    doc.h2("Spec checks");
    let mut c = Checks::new();
    header_checks(rom, &mut c);
    let secure = crc16(0xFFFF, &b[0x4000..0x8000.min(b.len())]);
    doc.p(&format!(
        "Secure-area CRC over the bytes as stored: {} vs header {} — {}.",
        hx(secure as u64, 4),
        hx(h.secure_area_checksum as u64, 4),
        if secure == h.secure_area_checksum {
            "match (dump is stored encrypted)"
        } else {
            "differs (dump is stored *decrypted*; the header CRC covers the encrypted form, which `Cartridge::encrypt_secure_area` recreates for BIOS boot)"
        }
    ));
    c.write(&mut doc);

    doc.h2("Raw header bytes (000h–17Fh)");
    doc.hexdump(0, &b[..0x180]);
    doc.save();
    c.finish();
}
