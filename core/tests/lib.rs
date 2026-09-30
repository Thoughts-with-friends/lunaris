//! Spec tests for `core/src/lib.rs` — foreign save-file normalisation — and
//! the generator of `core/tests/dist/README.md`, the index of every report.
//!
//! References: DeSmuME `.dsv` footer (`|-DESMUME SAVE-|`), no$gba save header.

use super::*;
use crate::test_support::{
    dist_path,
    md::{Doc, L},
    rom,
    spec::Checks,
};

fn dsv(raw: &[u8], banner: &[u8]) -> Vec<u8> {
    let mut v = raw.to_vec();
    v.extend_from_slice(banner);
    for w in [raw.len() as u32, 0x10, 0, 0, 0, 0] {
        v.extend_from_slice(&w.to_le_bytes());
    }
    v.extend_from_slice(DESMUME_FOOTER_COOKIE);
    v
}

#[test]
fn desmume_footer_is_stripped_and_raw_saves_pass_through() {
    let raw = vec![0xA5u8; 0x200];
    assert_eq!(normalize_foreign_save(&dsv(&raw, b"")), raw);
    assert_eq!(normalize_foreign_save(&dsv(&raw, b"banner text")), raw);
    assert_eq!(normalize_foreign_save(&raw), raw);
    let mut nocash = NOCASH_HEADER.to_vec();
    nocash.resize(0x400, 0);
    assert!(normalize_foreign_save(&nocash).is_empty());
}

/// Every report the suite writes, in reading order.
const REPORTS: &[(&str, &str, &str)] = &[
    ("Boot & frame loop", "nds.md", "frame loop, timing constants, synthetic ROM boot"),
    ("Boot & frame loop", "nds_timeline.md", "real ROM screens at fixed frames"),
    ("Boot & frame loop", "hw.md", "direct boot: memory state before the first instruction"),
    ("Boot & frame loop", "nds_savestate.md", "savestate round trip / deterministic replay"),
    ("Cartridge", "hw/cartridge/header.md", "the 200h-byte header, field by field"),
    ("Cartridge", "hw/cartridge.md", "FNT/FAT file system, overlays, banner icon, bus protocol"),
    ("Cartridge", "hw/cartridge/key1_encryption.md", "KEY1 Blowfish"),
    ("Cartridge", "hw/cartridge/backup.md", "save-chip detection, GAME_DB, IR routing"),
    ("Cartridge", "hw/cartridge/backup/eeprom.md", "EEPROM saves"),
    ("Cartridge", "hw/cartridge/backup/flash.md", "FLASH saves / firmware chip"),
    ("CPU", "arm.md", "condition codes, pipeline PC, IRQ entry"),
    ("CPU", "arm/arm.md", "ARM instruction set (both cores)"),
    ("CPU", "arm/thumb.md", "THUMB instruction set (both cores)"),
    ("CPU", "arm/registers.md", "modes and banked registers"),
    ("Memory", "hw/mem.md", "memory maps, mirroring, WRAMCNT"),
    ("Memory", "hw/mem/cp15.md", "CP15 and TCM"),
    ("System", "hw/scheduler.md", "event scheduler"),
    ("System", "hw/interrupt_controller.md", "IME / IE / IF"),
    ("System", "hw/timers.md", "timers"),
    ("System", "hw/dma.md", "DMA"),
    ("System", "hw/ipc.md", "IPC sync + FIFOs"),
    ("System", "hw/math.md", "divider and square root"),
    ("System", "hw/keypad.md", "keypad"),
    ("System", "hw/rtc.md", "real-time clock"),
    ("System", "hw/spi.md", "SPI bus, firmware header & user settings"),
    ("System", "hw/spi/tsc.md", "touchscreen controller"),
    ("System", "hw/ar.md", "Action Replay cheats"),
    ("Video", "hw/gpu.md", "LCD timing, DISPSTAT, POWCNT1"),
    ("Video", "hw/gpu/vram.md", "VRAM banks: every VRAMCNT mapping probed"),
    ("Video", "hw/gpu/vram_rom.md", "VRAM bank contents from the real ROM"),
    ("Video", "hw/gpu/engine2d.md", "tiles, maps, palettes, sprites, layer priority (PNG)"),
    ("Video", "hw/gpu/engine2d_rom.md", "real ROM: BG maps, palettes, tiles (PNG)"),
    ("Video", "hw/gpu/engine3d/geometry.md", "geometry commands, matrix stacks, viewport"),
    (
        "Video",
        "hw/gpu/engine3d/rendering.md",
        "3D layers: clear/opaque/translucent/alpha test, wireframe, depth (PNG)",
    ),
    (
        "Video",
        "hw/gpu/engine3d/rendering_rom.md",
        "real ROM 3D capture stopped after SWAP_BUFFERS (PNG)",
    ),
    ("Sound", "hw/spu.md", "sound channels and ADPCM (PNG waveform)"),
    ("Saves", "lib.md", "foreign save normalisation (.dsv / no$gba)"),
];

#[test]
fn report() {
    let mut c = Checks::new();
    let raw: Vec<u8> = (0..=255u8).collect();
    c.eq(
        "DeSmuME footer",
        "`|-DESMUME SAVE-|` footer removed, raw size from the footer",
        raw.len(),
        normalize_foreign_save(&dsv(&raw, b"")).len(),
    );
    c.eq(
        "DeSmuME + banner",
        "banner text between data and footer is dropped",
        raw.clone(),
        normalize_foreign_save(&dsv(&raw, b"some banner")),
    );
    c.eq("raw save", "no footer → bytes unchanged", raw.clone(), normalize_foreign_save(&raw));
    let mut nocash = NOCASH_HEADER.to_vec();
    nocash.resize(0x100, 0);
    c.eq(
        "no$gba",
        "unsupported format → treated as absent (empty)",
        0,
        normalize_foreign_save(&nocash).len(),
    );
    let mut doc = Doc::new("lib.md", "Foreign save files", "lib.rs");
    doc.source("synthetic save images built by the test");
    doc.code(
        "text",
        "DeSmuME .dsv\n\
         ┌────────────────────┬──────────────┬───────────────────────────────┬──────────────────┐\n\
         │ raw save (N bytes) │ banner text? │ footer: 6 × u32 (N, …)        │ \"|-DESMUME SAVE-|\" │\n\
         └────────────────────┴──────────────┴───────────────────────────────┴──────────────────┘\n\
         \x20                                    ▲ len - 40                     ▲ len - 16\n\
         normalize_foreign_save → first N bytes",
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();

    // ---- index -------------------------------------------------------------
    let mut out = String::from("# Lunaris core — specification reports\n\n");
    out += "Generated by `cargo test -p nds-core` (or the ▶ Run Test buttons in VS Code). \
            Each report mirrors one source file in `core/src/`: it explains the GBATEK \
            structure, shows real data in tables / hexdumps / PNGs, and ends with a ✅ / ⚠️ / ❌ \
            ledger of checks (⚠️ = documented gap in Lunaris, ❌ = regression).\n\n";
    out += &format!("**ROM used for real-data sections:** {}\n\n", rom::test_rom().label());
    let mut section = "";
    for &(sec, file, what) in REPORTS {
        if sec != section {
            out += &format!("\n## {sec}\n\n| Report | Contents |\n| --- | --- |\n");
            section = sec;
        }
        out += &format!("| [{file}]({file}) | {what} |\n");
    }
    out += &coverage();
    std::fs::write(dist_path("README.md"), out).unwrap();
    let _ = L;
}

/// `docs/ref-gbatek` chapter → reports that test it, plus what is not covered.
const CHAPTERS: &[(&str, &[&str], &str)] = &[
    ("01_emulator_architecture.md", &["nds.md", "nds_timeline.md"], ""),
    ("02_workspace_layout.md", &[], "code organisation only — nothing to execute"),
    (
        "03_arm_cpu.md",
        &["arm.md", "arm/arm.md", "arm/thumb.md", "arm/registers.md"],
        "cycle timings, SWI/undefined exceptions, coprocessor ops other than CP15",
    ),
    ("04_cp15_and_tcm.md", &["hw/mem/cp15.md"], "protection-unit regions, cache commands"),
    ("05_memory_map.md", &["hw/mem.md", "hw.md"], "GBA slot, access wait states"),
    ("06_scheduler_and_timers.md", &["hw/scheduler.md", "hw/timers.md"], ""),
    (
        "07_interrupts_and_ipc.md",
        &["hw/interrupt_controller.md", "hw/ipc.md"],
        "HALTCNT / IntrWait",
    ),
    (
        "08_dma.md",
        &["hw/dma.md"],
        "start-of-display, main-memory-display and GBA-slot timings (not implemented in Lunaris)",
    ),
    (
        "09_2d_engine.md",
        &["hw/gpu/engine2d.md", "hw/gpu/engine2d_rom.md"],
        "affine / bitmap / extended BGs, windows, BLDCNT blending, mosaic, master brightness",
    ),
    (
        "10_3d_geometry.md",
        &["hw/gpu/engine3d/geometry.md"],
        "lighting, box test, texture-coordinate transforms, clipping details",
    ),
    (
        "11_3d_rasterizer.md",
        &["hw/gpu/engine3d/rendering.md", "hw/gpu/engine3d/rendering_rom.md"],
        "texture-format sheet (only A5I3 exercised), fog, toon, edge marking, anti-aliasing",
    ),
    (
        "12_vram_and_display.md",
        &["hw/gpu.md", "hw/gpu/vram.md", "hw/gpu/vram_rom.md"],
        "display capture",
    ),
    ("13_spu.md", &["hw/spu.md"], "mixing, PSG/noise output, sound capture"),
    (
        "14_cartridge_and_boot.md",
        &["hw/cartridge/header.md", "hw/cartridge.md", "hw/cartridge/key1_encryption.md", "hw.md"],
        "KEY2 (not implemented in Lunaris), BIOS (non-direct) boot",
    ),
    (
        "15_backup_memory.md",
        &[
            "hw/cartridge/backup.md",
            "hw/cartridge/backup/eeprom.md",
            "hw/cartridge/backup/flash.md",
            "lib.md",
        ],
        "IR MCU command set beyond the 08h probe",
    ),
    ("16_spi_firmware_touchscreen.md", &["hw/spi.md", "hw/spi/tsc.md"], "power-management device"),
    (
        "17_rtc_keypad_math.md",
        &["hw/rtc.md", "hw/keypad.md", "hw/math.md"],
        "RTC alarms / write path",
    ),
    (
        "18_wifi_and_local_mp.md",
        &[],
        "**not covered**: Wi-Fi registers and local multiplayer need a second console / transport",
    ),
    ("19_savestates.md", &["nds_savestate.md"], "cross-version state compatibility"),
    (
        "20_cheats_debug_frontend.md",
        &["hw/ar.md"],
        "debug viewers and frontends (GUI is out of scope)",
    ),
];

fn coverage() -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let docs = root.join("../docs/ref-gbatek");
    let mut out = String::from(
        "\n## Coverage of `docs/ref-gbatek`\n\n| Chapter | Reports | Not covered |\n| --- | --- | --- |\n",
    );
    for (file, reports, gaps) in CHAPTERS {
        let title = std::fs::read_to_string(docs.join(file))
            .ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("# ")).map(|l| l[2..].to_string()))
            .unwrap_or_else(|| (*file).to_string());
        let links: Vec<String> = reports.iter().map(|r| format!("[{r}]({r})")).collect();
        out += &format!(
            "| [{title}](../../../docs/ref-gbatek/{file}) | {} | {gaps} |\n",
            if links.is_empty() { "–".into() } else { links.join("<br>") }
        );
    }
    // Source files that have no mirrored spec test.
    let mut missing = Vec::new();
    let src = root.join("src");
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let rel = p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
                if !root.join("tests").join(&rel).exists() {
                    missing.push(rel);
                }
            }
        }
    }
    missing.sort();
    out += &format!(
        "\n### `core/src` files without a mirrored test ({})\n\nMost are covered indirectly through their parent module's I/O-level tests (e.g. `mem/arm9/io.rs` by every register test); `hw/net/**` (Wi-Fi) is genuinely untested here.\n\n",
        missing.len()
    );
    out += &missing.iter().map(|m| format!("`{m}`")).collect::<Vec<_>>().join(", ");
    out.push('\n');
    out
}
