//! Spec tests for `core/src/hw/cartridge.rs` — the ROM file system (as the
//! game sees it through the cartridge bus) and the cartridge command
//! protocol (ROMCTRL / ROMCMD / ROMDATA).
//!
//! GBATEK:
//! - "DS Cartridge NitroROM and NitroARC File Systems":
//!   <https://problemkaputt.de/gbatek.htm#dscartridgenitroromandnitroarcfilesystems>
//! - "DS Cartridge Icon/Title": <https://problemkaputt.de/gbatek.htm#dscartridgeicontitle>
//! - "DS Cartridge Protocol": <https://problemkaputt.de/gbatek.htm#dscartridgeprotocol>
//! - "DS Cartridge I/O Ports": <https://problemkaputt.de/gbatek.htm#dscartridgeioports>

use super::*;
use crate::test_support::{
    boot::boot,
    crc16, le16, le32,
    md::{Doc, L, R, hx, size},
    png::{Canvas, WHITE},
    rom::{self, layout},
    spec::Checks,
};

// ---------------------------------------------------------------------------
// File system walkers (reference implementation straight from GBATEK)
// ---------------------------------------------------------------------------

/// One FNT entry.
#[derive(Debug, Clone)]
struct FsEntry {
    path: String,
    /// File id (FAT index) or directory id (F000h..).
    id: u16,
    is_dir: bool,
    depth: usize,
}

/// Walks the File Name Table depth-first starting at the root (F000h).
///
/// ```text
/// FNT main table: 8 bytes per directory, directory id = F000h + index
///   +0 u32  offset of this directory's sub-table (from FNT start)
///   +4 u16  id of the first file in this directory
///   +6 u16  root: total number of directories / others: parent id
/// sub-table: sequence of entries, terminated by 00h
///   01h..7Fh  file,      name length = type        (id = first_file++)
///   81h..FFh  directory, name length = type & 7Fh, followed by u16 dir id
/// ```
fn walk_fnt(rom: &[u8], fnt: usize) -> Vec<FsEntry> {
    fn dir(rom: &[u8], fnt: usize, id: u16, prefix: &str, depth: usize, out: &mut Vec<FsEntry>) {
        let main = fnt + (id as usize & 0xFFF) * 8;
        let mut p = fnt + le32(rom, main) as usize;
        let mut file_id = le16(rom, main + 4);
        loop {
            let t = rom[p];
            p += 1;
            if t == 0 || depth > 16 {
                break;
            }
            let len = (t & 0x7F) as usize;
            let name = String::from_utf8_lossy(&rom[p..p + len]).into_owned();
            p += len;
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if t & 0x80 != 0 {
                let sub = le16(rom, p);
                p += 2;
                out.push(FsEntry { path: path.clone(), id: sub, is_dir: true, depth });
                dir(rom, fnt, sub, &path, depth + 1, out);
            } else {
                out.push(FsEntry { path, id: file_id, is_dir: false, depth });
                file_id += 1;
            }
        }
    }
    let mut out = Vec::new();
    dir(rom, fnt, 0xF000, "", 0, &mut out);
    out
}

/// FAT: `(start, end)` ROM offsets per file id.
fn fat(rom: &[u8], off: usize, size: usize) -> Vec<(u32, u32)> {
    (0..size / 8).map(|i| (le32(rom, off + i * 8), le32(rom, off + i * 8 + 4))).collect()
}

/// One 32-byte overlay table entry.
#[derive(Debug)]
struct Overlay {
    id: u32,
    ram: u32,
    ram_size: u32,
    bss_size: u32,
    sinit_start: u32,
    sinit_end: u32,
    file_id: u32,
    flags: u32,
}

fn overlays(rom: &[u8], off: usize, size: usize) -> Vec<Overlay> {
    (0..size / 32)
        .map(|i| {
            let w = |k: usize| le32(rom, off + i * 32 + k * 4);
            Overlay {
                id: w(0),
                ram: w(1),
                ram_size: w(2),
                bss_size: w(3),
                sinit_start: w(4),
                sinit_end: w(5),
                file_id: w(6),
                flags: w(7),
            }
        })
        .collect()
}

/// Decodes the 32×32 4bpp banner icon to BGR555 pixels (colour 0 → magenta
/// checker so transparency is visible).
fn decode_icon(rom: &[u8], banner: usize) -> Vec<u16> {
    let pal: Vec<u16> = (0..16).map(|i| le16(rom, banner + 0x220 + i * 2)).collect();
    let mut px = vec![0u16; 32 * 32];
    for tile in 0..16 {
        for y in 0..8 {
            for x in 0..8 {
                let byte = rom[banner + 0x20 + tile * 32 + y * 4 + x / 2];
                let idx = if x % 2 == 0 { byte & 0xF } else { byte >> 4 } as usize;
                let (px_x, px_y) = ((tile % 4) * 8 + x, (tile / 4) * 8 + y);
                px[px_y * 32 + px_x] = if idx == 0 {
                    if (px_x / 4 + px_y / 4) % 2 == 0 { 0x7C1F } else { 0x5C17 }
                } else {
                    pal[idx]
                };
            }
        }
    }
    px
}

fn banner_title(rom: &[u8], banner: usize, lang: usize) -> String {
    let base = banner + 0x240 + lang * 0x100;
    let units: Vec<u16> =
        (0..0x80).map(|i| le16(rom, base + i * 2)).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

// ---------------------------------------------------------------------------
// Cartridge bus driver: issues commands exactly like game code does
// ---------------------------------------------------------------------------

/// Sends an 8-byte command through ROMCMD (40001A8h) and starts it via
/// ROMCTRL (40001A4h) with block size `bs` (1..6 → 200h << (bs-1), 7 → 4).
/// Returns every word read from ROMDATA (4100010h).
fn cart_command(hw: &mut HW, cmd: [u8; 8], bs: u32) -> Vec<u32> {
    // AUXSPICNT: slot enable (bit 15), ROM mode (bit 13 = 0).
    hw.arm9_write::<u16>(0x0400_01A0, 0x8000);
    for (i, b) in cmd.iter().enumerate() {
        hw.arm9_write::<u8>(0x0400_01A8 + i as u32, *b);
    }
    hw.arm9_write::<u32>(0x0400_01A4, 0x8000_0000 | bs << 24);
    let mut words = Vec::new();
    for _ in 0..0x2000 {
        let ctrl: u32 = hw.arm9_read(0x0400_01A4);
        if ctrl & 0x8000_0000 == 0 {
            break;
        }
        if ctrl & 0x0080_0000 != 0 {
            words.push(hw.arm9_read::<u32>(0x0410_0010));
        } else {
            let next = hw.cycle_at_next_event();
            hw.clock_until(next.max(hw.cycle() + 1));
        }
    }
    words
}

fn words_to_bytes(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|w| w.to_le_bytes()).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn synthetic_fnt_walk_finds_every_file_with_matching_fat_extent() {
    let r = &rom::synthetic().bytes;
    let entries = walk_fnt(r, layout::FNT_OFF);
    let files: Vec<_> = entries.iter().filter(|e| !e.is_dir).collect();
    assert_eq!(files.len(), 3);
    let table = fat(r, layout::FAT_OFF, layout::FILES.len() * 8);
    for f in files {
        let (name, contents) = layout::FILES[f.id as usize];
        assert_eq!(f.path, name);
        let (s, e) = table[f.id as usize];
        assert_eq!(&r[s as usize..e as usize], contents);
    }
    assert!(entries.iter().any(|e| e.is_dir && e.path == "data" && e.id == 0xF001));
}

#[test]
fn b7_data_read_returns_rom_bytes_at_the_big_endian_address() {
    let mut nds = boot(rom::synthetic(), "cart_b7");
    let hw = nds.hw_mut();
    let addr = layout::FILES_OFF as u32;
    let a = addr.to_be_bytes();
    let got = words_to_bytes(&cart_command(hw, [0xB7, a[0], a[1], a[2], a[3], 0, 0, 0], 1));
    assert_eq!(got.len(), 0x200);
    assert_eq!(got, &rom::synthetic().bytes[addr as usize..addr as usize + 0x200]);
}

#[test]
fn b7_reads_below_8000h_are_redirected_to_8000h_plus_low_bits() {
    let mut nds = boot(rom::synthetic(), "cart_b7_low");
    let got = words_to_bytes(&cart_command(nds.hw_mut(), [0xB7, 0, 0, 0x10, 0, 0, 0, 0], 7));
    let r = &rom::synthetic().bytes;
    assert_eq!(got, &r[0x8000..0x8004], "1000h → 8000h + (1000h & 1FFh)");
}

#[test]
fn b8_chip_id_repeats_for_the_whole_block() {
    let mut nds = boot(rom::synthetic(), "cart_b8");
    let got = cart_command(nds.hw_mut(), [0xB8, 0, 0, 0, 0, 0, 0, 0], 7);
    assert_eq!(got, vec![nds.hw().cartridge.chip_id()]);
}

#[test]
fn report() {
    let rom = rom::test_rom();
    let r = &rom.bytes;
    let h = Header::new(r);
    let mut c = Checks::new();
    let mut doc = Doc::new(
        "hw/cartridge.md",
        "Cartridge: NitroROM file system & bus protocol",
        "hw/cartridge.rs",
    );
    doc.source(&rom.label());

    // ---- FNT ----------------------------------------------------------------
    doc.h2("1. File Name Table (FNT)");
    doc.p(&format!(
        "Located by header `fnt_offset` = {} (`fnt_size` = {}). Two parts: a *main table* with one \
         8-byte record per directory, followed by the *sub-tables* holding the names.",
        hx(h.fnt_offset as u64, 8),
        size(h.fnt_size as u64)
    ));
    doc.table(
        &[("Field", L), ("Description", L), ("Rust Type", R), ("Size", R), ("Offset", R)],
        &[
            vec![
                "sub_table_offset",
                "Offset of this directory's name list, relative to FNT start",
                "u32",
                "4",
                "0",
            ],
            vec![
                "first_file_id",
                "FAT index of the first file listed in this directory",
                "u16",
                "2",
                "4",
            ],
            vec![
                "parent_or_count",
                "Root: total directory count. Others: parent directory id (F000h+)",
                "u16",
                "2",
                "6",
            ],
        ],
    );
    doc.p("Sub-table entry encoding:");
    doc.table(
        &[("Type byte", L), ("Meaning", L), ("Followed by", L)],
        &[
            vec!["00h", "end of this directory", "–"],
            vec!["01h..7Fh", "file, name length = type", "name (ASCII, no terminator)"],
            vec!["80h", "reserved", "–"],
            vec![
                "81h..FFh",
                "sub-directory, name length = type & 7Fh",
                "name, then u16 directory id (F001h..)",
            ],
        ],
    );
    let fnt = h.fnt_offset as usize;
    let dir_count = le16(r, fnt + 6) as usize;
    let mut main_rows = Vec::new();
    for d in 0..dir_count.min(8) {
        main_rows.push(vec![
            hx(0xF000 + d as u64, 4),
            hx(le32(r, fnt + d * 8) as u64, 8),
            le16(r, fnt + d * 8 + 4).to_string(),
            hx(le16(r, fnt + d * 8 + 6) as u64, 4),
        ]);
    }
    doc.h3(&format!("Main table ({dir_count} directories, first {} shown)", main_rows.len()));
    doc.table(
        &[("Dir id", L), ("sub_table_offset", R), ("first_file_id", R), ("parent / count", R)],
        &main_rows,
    );
    doc.h3("Root sub-table bytes");
    let root_sub = fnt + le32(r, fnt) as usize;
    doc.hexdump(root_sub as u64, &r[root_sub..(root_sub + 0x60).min(r.len())]);

    let entries = walk_fnt(r, fnt);
    let file_count = entries.iter().filter(|e| !e.is_dir).count();
    let dirs = entries.iter().filter(|e| e.is_dir).count() + 1;
    c.eq("FNT directory count", "root record +6 = number of directories", dir_count, dirs);
    doc.h3(&format!("Directory tree ({dirs} directories, {file_count} named files)"));
    let table = fat(r, h.fat_offset as usize, h.fat_size as usize);
    let mut tree = String::from("/ (F000h)\n");
    let mut shown_per_dir = std::collections::HashMap::<usize, usize>::new();
    for e in &entries {
        let n = shown_per_dir.entry(e.depth).or_default();
        if e.depth > 2 || (!e.is_dir && *n >= 6) {
            continue;
        }
        *n += 1;
        let indent = "│   ".repeat(e.depth);
        let name = e.path.rsplit('/').next().unwrap_or(&e.path);
        if e.is_dir {
            *shown_per_dir.entry(e.depth + 1).or_default() = 0;
            tree += &format!("{indent}├── {name}/  (dir {})\n", hx(e.id as u64, 4));
        } else {
            let (s, en) = table.get(e.id as usize).copied().unwrap_or((0, 0));
            tree += &format!(
                "{indent}├── {name}  (file #{}, ROM {}..{}, {})\n",
                e.id,
                hx(s as u64, 8),
                hx(en as u64, 8),
                size((en - s) as u64)
            );
        }
    }
    doc.code("text", &tree);
    doc.note(
        "At most 6 files per directory and 3 levels are drawn; the checks below cover every entry.",
    );

    // ---- FAT ----------------------------------------------------------------
    doc.h2("2. File Allocation Table (FAT)");
    doc.p("`fat_size / 8` records of `(start, end)` ROM offsets; `end` is exclusive. The file id is the index.");
    doc.code(
        "text",
        "fat_offset + id*8\n\
         ┌───────────────┬───────────────┐\n\
         │ start: u32    │ end: u32      │   size = end - start\n\
         └───────────────┴───────────────┘",
    );
    let fat_rows: Vec<Vec<String>> = table
        .iter()
        .enumerate()
        .take(12)
        .map(|(i, (s, e))| {
            let name =
                entries.iter().find(|x| !x.is_dir && x.id as usize == i).map(|x| x.path.clone());
            vec![
                i.to_string(),
                hx(*s as u64, 8),
                hx(*e as u64, 8),
                size((e - s) as u64),
                name.unwrap_or_else(|| "(overlay / unnamed)".into()),
            ]
        })
        .collect();
    doc.table(&[("id", R), ("start", R), ("end", R), ("size", R), ("path", L)], &fat_rows);
    let bad = table.iter().filter(|(s, e)| s > e || *e as usize > r.len()).count();
    c.eq("FAT extents valid", "start ≤ end ≤ ROM size for every file", 0, bad);
    let named_ok = entries.iter().filter(|e| !e.is_dir).all(|e| (e.id as usize) < table.len());
    c.ok(
        "FNT ids in FAT",
        "every FNT file id indexes the FAT",
        named_ok,
        format!("{} FAT entries", table.len()),
    );

    // ---- overlays -----------------------------------------------------------
    doc.h2("3. ARM9 overlay table (OVT)");
    doc.table(
        &[("Field", L), ("Description", L), ("Rust Type", R), ("Size", R), ("Offset", R)],
        &[
            vec!["overlay_id", "Overlay number", "u32", "4", "0x00"],
            vec!["ram_address", "Load address", "u32", "4", "0x04"],
            vec!["ram_size", "Bytes to load (from the FAT file)", "u32", "4", "0x08"],
            vec!["bss_size", "Bytes to zero after the loaded data", "u32", "4", "0x0C"],
            vec!["sinit_start", "Static-initialiser table start", "u32", "4", "0x10"],
            vec!["sinit_end", "Static-initialiser table end", "u32", "4", "0x14"],
            vec!["file_id", "FAT index holding the overlay data", "u32", "4", "0x18"],
            vec!["flags", "Bit 0-23 compressed size, bit 24 compressed flag", "u32", "4", "0x1C"],
        ],
    );
    let ovt = overlays(r, h.arm9_overlay_offset as usize, h.arm9_overlay_size as usize);
    let ov_rows: Vec<Vec<String>> = ovt
        .iter()
        .take(10)
        .map(|o| {
            vec![
                o.id.to_string(),
                hx(o.ram as u64, 8),
                size(o.ram_size as u64),
                size(o.bss_size as u64),
                format!("{}..{}", hx(o.sinit_start as u64, 8), hx(o.sinit_end as u64, 8)),
                o.file_id.to_string(),
                hx(o.flags as u64, 8),
            ]
        })
        .collect();
    doc.p(&format!("{} ARM9 overlays (first {} shown):", ovt.len(), ov_rows.len()));
    doc.table(
        &[
            ("id", R),
            ("ram", R),
            ("ram_size", R),
            ("bss", R),
            ("sinit", L),
            ("file", R),
            ("flags", R),
        ],
        &ov_rows,
    );
    c.ok(
        "overlay file ids",
        "every overlay file_id < FAT entry count",
        ovt.iter().all(|o| (o.file_id as usize) < table.len()),
        format!("{} overlays", ovt.len()),
    );

    // ---- banner -------------------------------------------------------------
    doc.h2("4. Banner (icon & titles)");
    let banner = h.icon_offset as usize;
    if banner != 0 && banner + 0x840 <= r.len() {
        doc.table(
            &[
                ("Field", L),
                ("Description", L),
                ("Rust Type", R),
                ("Size", R),
                ("Offset", R),
                ("Value", L),
            ],
            &[
                vec![
                    "version".into(),
                    "0001h (+ Chinese 0002h, + Korean 0003h)".into(),
                    "u16".into(),
                    "2".into(),
                    "0x000".into(),
                    hx(le16(r, banner) as u64, 4),
                ],
                vec![
                    "crc_v1".into(),
                    "CRC16 of 020h..83Fh".into(),
                    "u16".into(),
                    "2".into(),
                    "0x002".into(),
                    hx(le16(r, banner + 2) as u64, 4),
                ],
                vec![
                    "icon_bitmap".into(),
                    "32×32 4bpp, 4×4 tiles of 8×8".into(),
                    "[u8; 0x200]".into(),
                    "512".into(),
                    "0x020".into(),
                    String::new(),
                ],
                vec![
                    "icon_palette".into(),
                    "16 × BGR555, colour 0 transparent".into(),
                    "[u16; 16]".into(),
                    "32".into(),
                    "0x220".into(),
                    String::new(),
                ],
                vec![
                    "title[0..6]".into(),
                    "JP, EN, FR, DE, IT, ES; UTF-16, 128 chars each".into(),
                    "[[u16; 0x80]; 6]".into(),
                    "1536".into(),
                    "0x240".into(),
                    String::new(),
                ],
            ],
        );
        let crc = crc16(0xFFFF, &r[banner + 0x20..banner + 0x840]);
        c.hex(
            "banner CRC16",
            "banner+2 = CRC16(FFFFh, banner+20h..83Fh)",
            crc,
            le16(r, banner + 2),
        );
        let icon = decode_icon(r, banner);
        let mut img = Canvas::new(32 * 8 + 1, 32 * 8 + 1, WHITE);
        img.blit_bgr555(0, 0, &icon, 32, 32, 8);
        img.grid(64, [0, 0, 0, 255]);
        for t in 0..16 {
            img.label((t % 4) as i64 * 64 + 2, (t / 4) as i64 * 64 + 2, &format!("T{t}"), 2, WHITE);
        }
        img.save("hw/cartridge/icon.png");
        doc.p("Decoded icon, ×8, with the 4×4 tile grid (`T0`..`T15` = tile order in ROM; magenta checker = colour 0):");
        doc.image("icon", "cartridge/icon.png");
        let mut pal = Canvas::new(16 * 24, 24, WHITE);
        for i in 0..16 {
            pal.fill_rect(
                i * 24,
                0,
                24,
                24,
                crate::test_support::bgr555_to_rgba(le16(r, banner + 0x220 + i as usize * 2)),
            );
            pal.label(i * 24 + 2, 2, &format!("{i:X}"), 2, WHITE);
        }
        pal.save("hw/cartridge/icon_palette.png");
        doc.image("icon palette", "cartridge/icon_palette.png");
        let langs = ["Japanese", "English", "French", "German", "Italian", "Spanish"];
        let rows: Vec<Vec<String>> = langs
            .iter()
            .enumerate()
            .map(|(i, l)| {
                vec![
                    l.to_string(),
                    hx((banner + 0x240 + i * 0x100) as u64, 8),
                    banner_title(r, banner, i).replace('\n', " / "),
                ]
            })
            .collect();
        doc.table(
            &[("Language", L), ("ROM offset", R), ("Title (lines joined by ` / `)", L)],
            &rows,
        );
    }

    // ---- protocol -----------------------------------------------------------
    doc.h2("5. Cartridge bus protocol (as driven through I/O ports)");
    doc.p("Games never read the ROM directly: they write an 8-byte command to ROMCMD, start it through ROMCTRL, then pull 32-bit words from ROMDATA whenever bit 23 says one is ready.");
    doc.code(
        "text",
        "ARM9                          Cartridge (Lunaris `Cartridge`)\n\
         \x20│ 40001A8h..AFh ← B7 aa aa aa aa 00 00 00   (command, big-endian address)\n\
         \x20│ 40001A4h ← 8000_0000 | bs<<24          → run_command(): queue words\n\
         \x20│                                           schedule ROMWordTransfered\n\
         \x20│ poll 40001A4h bit23 (word ready) ◄──────── on_rom_word_transfered()\n\
         \x20│ read 4100010h  → next word ─────────────► schedule next word …\n\
         \x20│ …                                         last word → ROMBlockEnded\n\
         \x20│ 40001A4h bit31 = 0 (done) ◄─────────────── IRQ if AUXSPICNT bit14",
    );
    doc.bitfield(
        "40001A4h ROMCTRL",
        32,
        &[
            (31, 31, "Start"),
            (30, 30, "WR"),
            (29, 29, "RESB"),
            (28, 28, "GAPCLK"),
            (27, 27, "CLK"),
            (26, 24, "BLKSIZE"),
            (23, 23, "RDY"),
            (22, 22, "K2CMD"),
            (21, 16, "GAP2"),
            (15, 15, "SEED"),
            (13, 13, "K2DAT"),
            (12, 0, "KEY1 GAP1 LEN"),
        ],
    );
    doc.table(
        &[("BLKSIZE", R), ("Bytes", R)],
        &[
            vec!["0", "0"],
            vec!["1", "200h"],
            vec!["2", "400h"],
            vec!["3", "800h"],
            vec!["4", "1000h"],
            vec!["5", "2000h"],
            vec!["6", "4000h"],
            vec!["7", "4"],
        ],
    );
    doc.table(
        &[("Command", L), ("Mode", L), ("Meaning", L), ("Lunaris", L)],
        &[
            vec!["9F 00…", "unencrypted", "dummy (read FFh)", "`run_unencrypted_command`"],
            vec!["00 00…", "unencrypted", "read header (first 200h)", "copy_rom(0..)"],
            vec!["90 00…", "unencrypted", "1st chip ID", "chip_id repeated"],
            vec!["3C …", "unencrypted", "activate KEY1", "init_key_code(level 2)"],
            vec!["1x …", "KEY1", "2nd chip ID", "chip_id repeated"],
            vec!["2x …", "KEY1", "get secure area block (4000h..7FFFh)", "copy_rom(addr..+1000h)"],
            vec!["4x …", "KEY1", "activate KEY2", "FFh stream"],
            vec!["Ax …", "KEY1", "enter main data mode", "KEY1 off, zeros"],
            vec![
                "B7 aa aa aa aa 00 00 00",
                "KEY2",
                "read data (200h blocks, wraps within 4 KiB)",
                "`push_rom_data`",
            ],
            vec!["B8 00…", "KEY2", "3rd chip ID", "chip_id repeated"],
        ],
    );

    // Live transfer against the loaded ROM.
    let mut nds = boot(rom, "cart_report");
    let hw = nds.hw_mut();
    let probe = h.fat_offset & !0x1FF;
    let a = probe.to_be_bytes();
    let got = words_to_bytes(&cart_command(hw, [0xB7, a[0], a[1], a[2], a[3], 0, 0, 0], 1));
    c.ok(
        "B7 read 200h",
        "ROMDATA stream == ROM[addr..addr+200h]",
        got == r[probe as usize..probe as usize + 0x200],
        format!("{} bytes from {}", got.len(), hx(probe as u64, 8)),
    );
    let wrap_addr = (h.fat_offset | 0xF00) & !0xFF;
    let a = wrap_addr.to_be_bytes();
    let got = words_to_bytes(&cart_command(hw, [0xB7, a[0], a[1], a[2], a[3], 0, 0, 0], 1));
    let block = (wrap_addr & !0xFFF) as usize;
    let expect: Vec<u8> =
        (0..0x200).map(|i| r[block + ((wrap_addr as usize + i) & 0xFFF)]).collect();
    c.ok(
        "B7 4 KiB wrap",
        "reads wrap inside the current 4 KiB page",
        expect == got,
        format!("start {} → wraps to {}", hx(wrap_addr as u64, 8), hx(block as u64, 8)),
    );
    let chip = cart_command(hw, [0xB8, 0, 0, 0, 0, 0, 0, 0], 7);
    c.hex("B8 chip id", "one word = chip ID (Lunaris uses 00000FC2h)", 0x0000_0FC2u32, chip[0]);
    doc.p(&format!("Live transfer: `B7 {:08X}` returned 200h bytes starting with:", probe));
    doc.hexdump(probe as u64, &r[probe as usize..probe as usize + 0x40]);

    // ROMCTRL round trip of the KEY1 gap1 length (bits 0-12).
    hw.arm9_write::<u32>(0x0400_01A4, 0x1FFF);
    let back: u32 = hw.arm9_read(0x0400_01A4);
    c.known(
        "ROMCTRL gap1 read-back",
        "bits 0-12 = KEY1 gap1 length (R/W)",
        0x1FFFu32,
        back & 0x1FFF,
        "byte-1 write stores `(v & 1Fh) << 4` instead of `<< 8`",
    );

    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    let _ = layout::ARM9_OFF;
    c.finish();
}
