//! Spec tests for `core/src/hw/gpu/vram.rs` — the nine VRAM banks and their
//! VRAMCNT MST/OFS mappings.
//!
//! GBATEK "DS Memory Control - VRAM": <https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram>

use crate::{
    hw::{EngineA, EngineB, HW},
    test_support::{
        boot::{boot, frame_budget, io_machine, run_until},
        md::{Doc, L, R, hx, size},
        png::Canvas,
        rom,
        spec::Checks,
    },
};

const NAMES: [&str; 9] = ["A", "B", "C", "D", "E", "F", "G", "H", "I"];
const SIZES: [u32; 9] =
    [0x20000, 0x20000, 0x20000, 0x20000, 0x10000, 0x4000, 0x4000, 0x8000, 0x4000];
const LCDC: [u32; 9] = [
    0x0680_0000,
    0x0682_0000,
    0x0684_0000,
    0x0686_0000,
    0x0688_0000,
    0x0689_0000,
    0x0689_4000,
    0x0689_8000,
    0x068A_0000,
];

fn cnt_addr(bank: usize) -> u32 {
    match bank {
        0..=6 => 0x0400_0240 + bank as u32,
        7 => 0x0400_0248,
        _ => 0x0400_0249,
    }
}

/// Where a mapping lands, per GBATEK.
#[derive(Clone, Copy, Debug)]
enum Target {
    Arm9(u32, &'static str),
    Arm7(u32),
    Textures(usize),
    TexPal(usize),
    BgExtPalA(usize),
    ObjExtPalA,
    BgExtPalB(usize),
    ObjExtPalB,
}

/// Every legal `(bank, mst, ofs) → target` from GBATEK's VRAMCNT table.
fn mappings() -> Vec<(usize, u8, u8, Target)> {
    let mut v = Vec::new();
    let fg = |ofs: u8| 0x4000 * (ofs as u32 & 1) + 0x10000 * (ofs as u32 >> 1);
    for b in 0..4 {
        for ofs in 0..4u8 {
            v.push((b, 1, ofs, Target::Arm9(0x0600_0000 + 0x20000 * ofs as u32, "engine A BG")));
            v.push((b, 3, ofs, Target::Textures(ofs as usize)));
        }
    }
    for b in 0..2 {
        for ofs in 0..2u8 {
            v.push((b, 2, ofs, Target::Arm9(0x0640_0000 + 0x20000 * ofs as u32, "engine A OBJ")));
        }
    }
    for b in 2..4 {
        for ofs in 0..2u8 {
            v.push((b, 2, ofs, Target::Arm7(0x0600_0000 + 0x20000 * ofs as u32)));
        }
    }
    v.push((2, 4, 0, Target::Arm9(0x0620_0000, "engine B BG")));
    v.push((3, 4, 0, Target::Arm9(0x0660_0000, "engine B OBJ")));
    v.push((4, 1, 0, Target::Arm9(0x0600_0000, "engine A BG")));
    v.push((4, 2, 0, Target::Arm9(0x0640_0000, "engine A OBJ")));
    v.push((4, 3, 0, Target::TexPal(0)));
    v.push((4, 4, 0, Target::BgExtPalA(0)));
    for b in 5..7 {
        for ofs in 0..4u8 {
            v.push((b, 1, ofs, Target::Arm9(0x0600_0000 + fg(ofs), "engine A BG")));
            v.push((b, 2, ofs, Target::Arm9(0x0640_0000 + fg(ofs), "engine A OBJ")));
            v.push((b, 3, ofs, Target::TexPal(((ofs & 1) + (ofs >> 1) * 4) as usize)));
        }
        for ofs in 0..2u8 {
            v.push((b, 4, ofs, Target::BgExtPalA(ofs as usize * 2)));
        }
        v.push((b, 5, 0, Target::ObjExtPalA));
    }
    v.push((7, 1, 0, Target::Arm9(0x0620_0000, "engine B BG")));
    v.push((7, 2, 0, Target::BgExtPalB(0)));
    v.push((8, 1, 0, Target::Arm9(0x0620_8000, "engine B BG")));
    v.push((8, 2, 0, Target::Arm9(0x0660_0000, "engine B OBJ")));
    v.push((8, 3, 0, Target::ObjExtPalB));
    v
}

/// Writes `marker` through LCDC, remaps, reads it back at the target.
fn probe(hw: &mut HW, bank: usize, mst: u8, ofs: u8, target: Target, marker: u16) -> Option<u16> {
    for b in 0..9 {
        hw.arm9_write::<u8>(cnt_addr(b), 0);
    }
    hw.arm9_write::<u8>(cnt_addr(bank), 0x80);
    hw.arm9_write::<u16>(LCDC[bank], marker);
    hw.arm9_write::<u8>(cnt_addr(bank), 0x80 | ofs << 3 | mst);
    let v = &hw.gpu.vram;
    Some(match target {
        Target::Arm9(a, _) => hw.arm9_read::<u16>(a),
        Target::Arm7(a) => hw.arm7_read::<u16>(a),
        Target::Textures(slot) => v.get_textures::<u16>(slot * 0x20000),
        Target::TexPal(slot) => v.get_textures_pal::<u16>(slot * 0x4000),
        Target::BgExtPalA(slot) => v.get_bg_ext_pal::<EngineA>(slot, 0),
        Target::ObjExtPalA => v.get_obj_ext_pal::<EngineA>(0),
        Target::BgExtPalB(slot) => v.get_bg_ext_pal::<EngineB>(slot, 0),
        Target::ObjExtPalB => v.get_obj_ext_pal::<EngineB>(0),
    })
}

fn describe(t: Target) -> (String, String) {
    match t {
        Target::Arm9(a, what) => (what.into(), format!("ARM9 {}", hx(a as u64, 8))),
        Target::Arm7(a) => ("ARM7 WRAM".into(), format!("ARM7 {}", hx(a as u64, 8))),
        Target::Textures(s) => {
            ("3D texture image".into(), format!("slot {s} ({})", hx(s as u64 * 0x20000, 5)))
        }
        Target::TexPal(s) => {
            ("3D texture palette".into(), format!("slot {s} ({})", hx(s as u64 * 0x4000, 5)))
        }
        Target::BgExtPalA(s) => ("engine A BG ext. palette".into(), format!("slot {s}")),
        Target::ObjExtPalA => ("engine A OBJ ext. palette".into(), "–".into()),
        Target::BgExtPalB(s) => ("engine B BG ext. palette".into(), format!("slot {s}")),
        Target::ObjExtPalB => ("engine B OBJ ext. palette".into(), "–".into()),
    }
}

#[test]
fn every_gbatek_mapping_exposes_the_bank_at_the_documented_place() {
    let mut nds = io_machine("vram_map");
    let hw = nds.hw_mut();
    for (i, (bank, mst, ofs, t)) in mappings().into_iter().enumerate() {
        let marker = 0x1000 + i as u16;
        assert_eq!(
            probe(hw, bank, mst, ofs, t, marker),
            Some(marker),
            "bank {} MST {mst} OFS {ofs} → {t:?}",
            NAMES[bank]
        );
    }
}

#[test]
fn report() {
    let mut nds = io_machine("vram_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/gpu/vram.md", "VRAM banks and VRAMCNT", "hw/gpu/vram.rs");
    doc.source("register-level probe: a marker is written through LCDC, the bank is remapped, and the marker is read back where GBATEK says it must appear");
    doc.bitfield("VRAMCNT_x (4000240h–4000249h)", 8, &[(7, 7, "EN"), (4, 3, "OFS"), (2, 0, "MST")]);
    let rows: Vec<Vec<String>> = (0..9)
        .map(|b| {
            vec![
                NAMES[b].into(),
                size(SIZES[b] as u64),
                hx(cnt_addr(b) as u64, 8),
                hx(LCDC[b] as u64, 8),
            ]
        })
        .collect();
    doc.table(&[("Bank", L), ("Size", R), ("VRAMCNT", R), ("LCDC address (MST 0)", R)], &rows);
    doc.code(
        "text",
        "ARM9 view of VRAM (MST ≠ 0):\n\
         06000000 ┌──────────────── engine A BG  (A–D: +20000h·OFS, E, F/G: +4000h·(OFS&1)+10000h·(OFS>>1)) ┐\n\
         06200000 ├──────────────── engine B BG  (C MST4, H MST1, I MST1 at +8000h)                        │\n\
         06400000 ├──────────────── engine A OBJ (A/B: +20000h·(OFS&1), E, F/G)                            │\n\
         06600000 ├──────────────── engine B OBJ (D MST4, I MST2)                                           │\n\
         06800000 └──────────────── LCDC: every bank at a fixed address (MST 0)                            ┘\n\
         not CPU-visible: 3D texture slots 0-3 (A–D MST3), texture palette (E/F/G MST3),\n\
         \x20                extended palettes (E/F/G MST4/5, H MST2, I MST3)\n\
         ARM7 06000000+20000h·OFS: C/D with MST2",
    );
    let mut c = Checks::new();
    let mut rows = Vec::new();
    for (i, (bank, mst, ofs, t)) in mappings().into_iter().enumerate() {
        let marker = 0x2000 + i as u16;
        let got = probe(hw, bank, mst, ofs, t, marker);
        let (what, where_) = describe(t);
        let ok = got == Some(marker);
        c.ok(
            &format!("{} MST{mst} OFS{ofs}", NAMES[bank]),
            &format!("{what} @ {where_}"),
            ok,
            format!("{got:04X?}"),
        );
        rows.push(vec![
            NAMES[bank].into(),
            mst.to_string(),
            ofs.to_string(),
            what,
            where_,
            if ok { "✅" } else { "❌" }.into(),
        ]);
    }
    doc.h2("Mapping table (probed)");
    doc.table(
        &[("Bank", L), ("MST", R), ("OFS", R), ("Target", L), ("Location", L), ("", L)],
        &rows,
    );
    for b in 0..9 {
        hw.arm9_write::<u8>(cnt_addr(b), 0x80 | 0x18);
    }
    let back: Vec<u8> = (0..9).map(|b| hw.arm9_read::<u8>(cnt_addr(b))).collect();
    c.eq(
        "VRAMCNT read-back",
        "EN|OFS|MST as written (MST0 ignores OFS on real HW)",
        vec![0x98u8; 9],
        back,
    );
    doc.h2("Spec checks (summary)");
    let mut summary = Checks::new();
    let n = rows.len();
    let ok = rows.iter().filter(|r| r[5] == "✅").count();
    summary.eq("mappings", "all probed mappings land where GBATEK says", n, ok);
    summary.write(&mut doc);
    doc.save();
    c.finish();
    summary.finish();
}

#[test]
fn real_rom_banks() {
    let rom = rom::test_rom();
    let mut nds = boot(rom, "vram_rom");
    run_until(&mut nds, frame_budget(600), |_| false);
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/gpu/vram_rom.md", "VRAM banks: real ROM snapshot", "hw/gpu/vram.rs");
    doc.source(&format!("{} after {} frames", rom.label(), frame_budget(600)));
    let mut rows = Vec::new();
    for b in 0..9 {
        let v = hw.arm9_read::<u8>(cnt_addr(b));
        let (px, w, h) = hw.render_bank(true, b);
        let file = format!("hw/gpu/vram_rom/bank_{}.png", NAMES[b]);
        if w > 0 && h > 0 && v & 0x80 != 0 {
            Canvas::from_bgr555(&px, w, h, 1).save(&file);
        }
        rows.push(vec![
            NAMES[b].into(),
            hx(v as u64, 2),
            (v >> 7).to_string(),
            (v & 7).to_string(),
            (v >> 3 & 3).to_string(),
            if v & 0x80 != 0 {
                format!("![{}]({})", NAMES[b], &file["hw/gpu/".len()..])
            } else {
                String::new()
            },
        ]);
    }
    doc.p("Each bank viewed as raw BGR555 pixels (`VRAM::render_bank`), i.e. exactly what LCDC mode would show — bitmaps look like pictures, tile data looks like noise:");
    doc.table(
        &[
            ("Bank", L),
            ("VRAMCNT", R),
            ("EN", R),
            ("MST", R),
            ("OFS", R),
            ("Contents as BGR555", L),
        ],
        &rows,
    );
    doc.save();
}
