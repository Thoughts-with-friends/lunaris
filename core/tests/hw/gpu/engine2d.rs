//! Spec tests for `core/src/hw/gpu/engine2d.rs` — tiled backgrounds, the
//! tile/map/palette formats, sprites and layer priority.
//!
//! GBATEK:
//! - "DS Video BG Modes / Control": <https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol>
//! - "LCD VRAM Character Data": <https://problemkaputt.de/gbatek.htm#lcdvramcharacterdata>
//! - "LCD VRAM BG Screen Data Format (BG Map)": <https://problemkaputt.de/gbatek.htm#lcdvrambgscreendataformatbgmap>
//! - "LCD OBJ - OAM Attributes": <https://problemkaputt.de/gbatek.htm#lcdobjoamattributes>
//! - "DS Video OBJs": <https://problemkaputt.de/gbatek.htm#dsvideoobjs>

use crate::{
    NDS,
    hw::{Engine, GraphicsType, HW},
    nds::{HEIGHT, WIDTH},
    test_support::{
        boot::{boot, frame_budget, io_machine, run_until},
        md::{Doc, L, R, hx},
        png::{self, BLACK, Canvas, WHITE, glyph},
        rom,
        spec::Checks,
    },
};

// ---------------------------------------------------------------------------
// Synthetic scene data (the "spec" side: what GBATEK says the bytes mean)
// ---------------------------------------------------------------------------

const BG_PAL: u32 = 0x0500_0000;
const OBJ_PAL: u32 = 0x0500_0200;
const OAM: u32 = 0x0700_0000;
const BG_VRAM: u32 = 0x0600_0000;
const OBJ_VRAM: u32 = 0x0640_0000;
const BG0_MAP: u32 = BG_VRAM + 0x800; // screen base 1
const BG1_CHARS: u32 = BG_VRAM + 0x8000; // char base 2
const BG1_MAP: u32 = BG_VRAM + 0x1000; // screen base 2
const BG1_HOFS: usize = 4;
const BG1_VOFS: usize = 2;
const BACKDROP: u16 = 0x1084;

/// 4bpp tile `n`: hex digit `n` in colour 1, top/left border in colour 2,
/// everything else colour 0 (transparent).
fn tile4(n: usize) -> [[u8; 8]; 8] {
    let mut t = [[0u8; 8]; 8];
    let g = glyph(char::from_digit(n as u32, 16).unwrap().to_ascii_uppercase());
    for (y, row) in g.iter().enumerate() {
        for x in 0..3 {
            if row & (0b100 >> x) != 0 {
                t[y + 2][x + 3] = 1;
            }
        }
    }
    for i in 0..8 {
        t[0][i] = 2;
        t[i][0] = 2;
    }
    t
}

/// 4bpp packing: two pixels per byte, **low nibble = left pixel**.
fn pack4(t: &[[u8; 8]; 8]) -> [u8; 32] {
    let mut b = [0u8; 32];
    for y in 0..8 {
        for x in (0..8).step_by(2) {
            b[y * 4 + x / 2] = t[y][x] | t[y][x + 1] << 4;
        }
    }
    b
}

/// 8bpp tile `n` for BG1: diagonal gradient over palette entries 128..255.
fn tile8(n: usize) -> [[u8; 8]; 8] {
    let mut t = [[0u8; 8]; 8];
    for y in 0..8 {
        for x in 0..8 {
            t[y][x] = 128 + (((x + y) * 4 + n * 32) % 128) as u8;
        }
    }
    t
}

/// BG0 map entry at tile (x, y).
fn bg0_entry(x: usize, y: usize) -> u16 {
    let tile = ((x + y) % 16) as u16;
    let hflip = (y % 2 == 1) as u16;
    let vflip = (x % 3 == 0) as u16;
    let pal = ((y / 4) % 4 + 1) as u16;
    tile | hflip << 10 | vflip << 11 | pal << 12
}

fn bg1_entry(x: usize, y: usize) -> u16 {
    ((x / 2 + y / 2) % 4) as u16 | ((x / 4 % 2) as u16) << 10
}

fn bg_palette() -> Vec<u16> {
    let mut p = vec![0u16; 256];
    p[0] = BACKDROP;
    for bank in 1..5 {
        let [r, g, b, _] = png::hsv(bank as f32 * 0.21, 0.7, 1.0);
        p[bank * 16 + 1] = (r as u16 >> 3) | (g as u16 >> 3) << 5 | (b as u16 >> 3) << 10;
        p[bank * 16 + 2] = 0x5294;
    }
    for i in 128..256 {
        let t = (i - 128) as u16 / 4;
        p[i] = (t / 2) | (8 + t / 3) << 5 | (31 - t / 2) << 10;
    }
    p
}

/// Reference decoder for a text BG pixel, straight from GBATEK.
fn ref_pixel(
    pal: &[u16],
    entry: u16,
    tile: &[[u8; 8]; 8],
    px: usize,
    py: usize,
    bpp8: bool,
) -> Option<u16> {
    let (mut tx, mut ty) = (px % 8, py % 8);
    if entry & 1 << 10 != 0 {
        tx = 7 - tx;
    }
    if entry & 1 << 11 != 0 {
        ty = 7 - ty;
    }
    let idx = tile[ty][tx] as usize;
    if idx == 0 {
        return None;
    }
    Some(if bpp8 { pal[idx] } else { pal[(entry >> 12) as usize * 16 + idx] })
}

fn ball_tile_pixel(x: usize, y: usize) -> u8 {
    let (dx, dy) = (x as i32 - 15, y as i32 - 15);
    let r2 = dx * dx + dy * dy;
    if r2 > 225 {
        0
    } else if r2 > 150 {
        1
    } else if dx + dy < -6 {
        3
    } else {
        2
    }
}

fn upload_scene(hw: &mut HW) {
    hw.arm9_write::<u32>(0x0400_0304, 0x8203);
    hw.arm9_write::<u8>(0x0400_0240, 0x81); // bank A → BG
    hw.arm9_write::<u8>(0x0400_0241, 0x82); // bank B → OBJ
    for (i, c) in bg_palette().iter().enumerate() {
        hw.arm9_write::<u16>(BG_PAL + i as u32 * 2, *c);
    }
    for (i, c) in [0u16, 0x0C63, 0x03FF, 0x7FFF].iter().enumerate() {
        hw.arm9_write::<u16>(OBJ_PAL + i as u32 * 2, *c);
    }
    for n in 0..16 {
        let b = pack4(&tile4(n));
        for k in (0..32).step_by(2) {
            hw.arm9_write::<u16>(
                BG_VRAM + n as u32 * 32 + k as u32,
                b[k] as u16 | (b[k + 1] as u16) << 8,
            );
        }
    }
    for n in 0..4 {
        let t = tile8(n);
        for y in 0..8 {
            for x in (0..8).step_by(2) {
                hw.arm9_write::<u16>(
                    BG1_CHARS + (n * 64 + y * 8 + x) as u32,
                    t[y][x] as u16 | (t[y][x + 1] as u16) << 8,
                );
            }
        }
    }
    for y in 0..32 {
        for x in 0..32 {
            hw.arm9_write::<u16>(BG0_MAP + ((y * 32 + x) * 2) as u32, bg0_entry(x, y));
            hw.arm9_write::<u16>(BG1_MAP + ((y * 32 + x) * 2) as u32, bg1_entry(x, y));
        }
    }
    // 32×32 4bpp ball = 16 tiles, 1D mapping.
    for ty in 0..4 {
        for tx in 0..4 {
            for y in 0..8 {
                for x in (0..8).step_by(2) {
                    let p = |x: usize| ball_tile_pixel(tx * 8 + x, ty * 8 + y) as u16;
                    let addr = OBJ_VRAM + ((ty * 4 + tx) * 32 + y * 4 + x / 2) as u32;
                    // Two bytes (four pixels) per halfword write.
                    if x % 4 == 0 {
                        hw.arm9_write::<u16>(
                            addr,
                            p(x) | p(x + 1) << 4 | p(x + 2) << 8 | p(x + 3) << 12,
                        );
                    }
                }
            }
        }
    }
    for i in 0..128 {
        hw.arm9_write::<u16>(OAM + i * 8, 0x0200); // disabled
    }
    let sprite = |hw: &mut HW, i: u32, x: u16, y: u16, flags: u16, prio: u16| {
        hw.arm9_write::<u16>(OAM + i * 8, y);
        hw.arm9_write::<u16>(OAM + i * 8 + 2, x | flags | 2 << 14);
        hw.arm9_write::<u16>(OAM + i * 8 + 4, prio << 10);
    };
    sprite(hw, 0, 176, 96, 0, 0);
    sprite(hw, 1, 200, 20, 1 << 12, 1);
    hw.arm9_write::<u16>(0x0400_0008, 0x0100); // BG0CNT: prio 0, char 0, screen 1, 4bpp
    hw.arm9_write::<u16>(0x0400_000A, 0x0289); // BG1CNT: prio 1, char 2, 8bpp, screen 2
    hw.arm9_write::<u16>(0x0400_0014, BG1_HOFS as u16);
    hw.arm9_write::<u16>(0x0400_0016, BG1_VOFS as u16);
}

const DISP_BASE: u32 = 0x0001_0010; // graphics display, BG mode 0, OBJ 1D
const BG0: u32 = 1 << 8;
const BG1: u32 = 1 << 9;
const OBJ: u32 = 1 << 12;

fn frame_with(nds: &mut NDS, layers: u32) -> Vec<u16> {
    nds.hw_mut().arm9_write::<u32>(0x0400_0000, DISP_BASE | layers);
    nds.emulate_frame();
    nds.get_screens()[0].iter().map(|p| p & 0x7FFF).collect()
}

fn scene_machine(name: &str) -> NDS {
    let mut nds = io_machine(name);
    run_until(&mut nds, 1, |_| false); // let the synthetic ARM9 program finish
    upload_scene(nds.hw_mut());
    nds
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn bg0_text_layer_matches_reference_decoder() {
    let mut nds = scene_machine("2d_bg0");
    let got = frame_with(&mut nds, BG0);
    let pal = bg_palette();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let e = bg0_entry(x / 8, y / 8);
            let want =
                ref_pixel(&pal, e, &tile4((e & 0x3FF) as usize), x, y, false).unwrap_or(BACKDROP);
            assert_eq!(got[y * WIDTH + x], want, "BG0 pixel ({x},{y}) entry {e:04X}");
        }
    }
}

#[test]
fn bg1_8bpp_layer_with_scroll_matches_reference_decoder() {
    let mut nds = scene_machine("2d_bg1");
    let got = frame_with(&mut nds, BG1);
    let pal = bg_palette();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (sx, sy) = ((x + BG1_HOFS) % 256, (y + BG1_VOFS) % 256);
            let e = bg1_entry(sx / 8, sy / 8);
            let want =
                ref_pixel(&pal, e, &tile8((e & 0x3FF) as usize), sx, sy, true).unwrap_or(BACKDROP);
            assert_eq!(got[y * WIDTH + x], want, "BG1 pixel ({x},{y})");
        }
    }
}

fn label_cells(img: &mut Canvas, scale: usize) {
    img.grid(8 * scale, [255, 255, 255, 90]);
    for ty in 0..HEIGHT / 8 {
        for tx in 0..WIDTH / 8 {
            let e = bg0_entry(tx, ty);
            let mut s = format!("{}", e >> 12);
            if e & 1 << 10 != 0 {
                s.push('H');
            }
            if e & 1 << 11 != 0 {
                s.push('V');
            }
            img.label(
                (tx * 8 * scale + 1) as i64,
                (ty * 8 * scale + scale * 8 - 7) as i64,
                &s,
                1,
                WHITE,
            );
        }
    }
}

#[test]
fn report() {
    let mut nds = scene_machine("2d_report");
    let mut doc = Doc::new(
        "hw/gpu/engine2d.md",
        "2D engine: tiles, maps, palettes, sprites",
        "hw/gpu/engine2d.rs",
    );
    doc.source("synthetic scene uploaded through VRAM / palette / OAM (`upload_scene`) and rendered by real frames");
    doc.p("The 2D engine composes up to four backgrounds and 128 sprites per scanline. A *text* background is a grid of 8×8 **tiles** (character data) picked by a **map** (screen data) and coloured through a **palette**:");
    doc.code(
        "text",
        "screen pixel (x, y)\n\
         \x20  │  + BGnHOFS/VOFS scroll\n\
         \x20  ▼\n\
         map entry = VRAM[screen_base·800h + ((y/8)·32 + x/8)·2]      (u16, see below)\n\
         \x20  │ tile number, H/V flip, palette bank\n\
         \x20  ▼\n\
         texel = VRAM[char_base·4000h + tile·32 (4bpp) or ·64 (8bpp) + row/col]\n\
         \x20  │ 0 = transparent\n\
         \x20  ▼\n\
         colour = palette[bank·16 + texel] (4bpp) or palette[texel] (8bpp)   → BGR555",
    );
    doc.h2("Registers used by the scene");
    doc.bitfield(
        "4000000h DISPCNT (engine A)",
        32,
        &[
            (31, 31, "OBJ EXT"),
            (30, 30, "BG EXT"),
            (29, 27, "SCR BASE"),
            (26, 24, "CHR BASE"),
            (23, 23, "HBLK OBJ"),
            (21, 20, "OBJ 1D BND"),
            (17, 16, "DISP MODE"),
            (15, 13, "WIN"),
            (12, 12, "OBJ"),
            (11, 8, "BG3-0"),
            (4, 4, "OBJ 1D"),
            (3, 3, "BG0 3D"),
            (2, 0, "BG MODE"),
        ],
    );
    doc.bitfield(
        "4000008h BGnCNT",
        16,
        &[
            (15, 14, "SIZE"),
            (13, 13, "WRAP/EXT"),
            (12, 8, "SCREEN BASE"),
            (7, 7, "8BPP"),
            (6, 6, "MOS"),
            (5, 2, "CHAR BASE"),
            (1, 0, "PRIO"),
        ],
    );
    doc.bitfield(
        "Text BG map entry (u16)",
        16,
        &[(15, 12, "PALETTE"), (11, 11, "V"), (10, 10, "H"), (9, 0, "TILE NUMBER")],
    );
    let hw = nds.hw_mut();
    let regs = [(0x0400_0008, "BG0CNT"), (0x0400_000A, "BG1CNT")];
    let rows: Vec<Vec<String>> = regs
        .iter()
        .map(|&(a, n)| {
            let v = hw.arm9_read::<u16>(a);
            vec![
                n.into(),
                hx(v as u64, 4),
                (v & 3).to_string(),
                hx((v as u64 >> 2 & 0xF) * 0x4000, 5),
                hx((v as u64 >> 8 & 0x1F) * 0x800, 5),
                if v & 0x80 != 0 { "8bpp (256 colours)" } else { "4bpp (16×16)" }.into(),
                format!("{}×256", 256 << (v >> 14 & 1)),
            ]
        })
        .collect();
    doc.table(
        &[
            ("Reg", L),
            ("Value", R),
            ("Prio", R),
            ("Char base", R),
            ("Screen base", R),
            ("Colours", L),
            ("Size", L),
        ],
        &rows,
    );

    // ---- tiles --------------------------------------------------------------
    doc.h2("Character data (tiles)");
    doc.code(
        "text",
        "4bpp tile = 32 bytes: 8 rows × 4 bytes, low nibble = LEFT pixel\n\
         \x20 byte:   0      1      2      3\n\
         \x20 row 0 [p1|p0] [p3|p2] [p5|p4] [p7|p6]\n\
         \x20 row 1 [p1|p0] …\n\
         8bpp tile = 64 bytes: 8 rows × 8 bytes, one byte per pixel",
    );
    let mut sheet = Canvas::new(16 * 8 * 6 + 1, 8 * 6 + 12, WHITE);
    let pal = bg_palette();
    for n in 0..16 {
        let t = tile4(n);
        for y in 0..8 {
            for x in 0..8 {
                let c = match t[y][x] {
                    0 => {
                        if (x + y) % 2 == 0 {
                            [220, 220, 220, 255]
                        } else {
                            [190, 190, 190, 255]
                        }
                    }
                    i => crate::test_support::bgr555_to_rgba(pal[16 + i as usize]),
                };
                sheet.fill_rect((n * 48 + x * 6) as i64, (12 + y * 6) as i64, 6, 6, c);
            }
        }
        sheet.rect((n * 48) as i64, 12, 49, 49, BLACK);
        sheet.text((n * 48 + 18) as i64, 2, &format!("{n:X}"), 2, BLACK);
    }
    sheet.save("hw/gpu/engine2d/tiles4.png");
    doc.p("Tiles 0–F at ×6 (checker = colour 0, transparent). Each shows its own number, so flips are visible in the map below:");
    doc.image("tiles", "engine2d/tiles4.png");
    let b = pack4(&tile4(1));
    doc.p("Bytes of tile 1 exactly as stored in VRAM:");
    doc.hexdump(BG_VRAM as u64 + 32, &b);

    // ---- map + layers --------------------------------------------------------
    let bg0 = frame_with(&mut nds, BG0);
    let bg1 = frame_with(&mut nds, BG1);
    let obj = frame_with(&mut nds, OBJ);
    let all = frame_with(&mut nds, BG0 | BG1 | OBJ);
    doc.h2("BG0 map, annotated");
    doc.p("BG0 rendered alone at ×3 with the 8×8 tile grid. Each cell is labelled with its palette bank and `H`/`V` flip flags taken from the map entry: rows alternate H-flip, every third column is V-flipped, the bank changes every 4 rows.");
    let mut annotated = Canvas::from_bgr555(&bg0, WIDTH, HEIGHT, 3);
    label_cells(&mut annotated, 3);
    annotated.save("hw/gpu/engine2d/bg0_map_annotated.png");
    doc.image("bg0 annotated", "engine2d/bg0_map_annotated.png");
    let map_rows: Vec<Vec<String>> = (0..4)
        .map(|y| (0..8).map(|x| format!("`{:04X}`", bg0_entry(x, y))).collect::<Vec<_>>())
        .map(|mut r| {
            r.insert(0, String::new());
            r
        })
        .enumerate()
        .map(|(y, mut r)| {
            r[0] = format!("row {y}");
            r
        })
        .collect();
    doc.p("First map entries (u16) — e.g. `1C00` = tile 0, V-flip, palette 1:");
    doc.table(
        &[
            ("", L),
            ("x0", L),
            ("x1", L),
            ("x2", L),
            ("x3", L),
            ("x4", L),
            ("x5", L),
            ("x6", L),
            ("x7", L),
        ],
        &map_rows,
    );

    doc.h2("Layer decomposition");
    let c = |p: &Vec<u16>| Canvas::from_bgr555(p, WIDTH, HEIGHT, 1);
    let layers = {
        let items = [
            ("BG0 PRIO0 4BPP", c(&bg0)),
            ("BG1 PRIO1 8BPP", c(&bg1)),
            ("OBJ", c(&obj)),
            ("COMPOSITE", c(&all)),
        ];
        let mut out = Canvas::new(4 * (WIDTH + 6), HEIGHT + 16, WHITE);
        for (i, (t, img)) in items.iter().enumerate() {
            out.blit((i * (WIDTH + 6)) as i64, 16, img);
            out.text((i * (WIDTH + 6)) as i64 + 2, 4, t, 2, BLACK);
        }
        out
    };
    layers.save("hw/gpu/engine2d/layers.png");
    doc.image("layers", "engine2d/layers.png");
    doc.code(
        "text",
        "front ─────────────────────────────────────────────► back\n\
         OBJ prio0 > BG0 prio0 > OBJ prio1 > BG1 prio1 > … > backdrop (palette[0])\n\
         (same priority: OBJ beats BG; BG0 beats BG1)",
    );

    // ---- sprites -------------------------------------------------------------
    doc.h2("Sprites (OAM)");
    doc.bitfield(
        "OAM attribute 0",
        16,
        &[
            (15, 14, "SHAPE"),
            (13, 13, "8BPP"),
            (12, 12, "MOS"),
            (11, 10, "MODE"),
            (9, 9, "DIS/DBL"),
            (8, 8, "AFF"),
            (7, 0, "Y"),
        ],
    );
    doc.bitfield(
        "OAM attribute 1",
        16,
        &[(15, 14, "SIZE"), (13, 13, "V"), (12, 12, "H"), (8, 0, "X")],
    );
    doc.bitfield("OAM attribute 2", 16, &[(15, 12, "PALETTE"), (11, 10, "PRIO"), (9, 0, "TILE")]);
    let oam: Vec<Vec<String>> = (0..2u32)
        .map(|i| {
            let a: Vec<u16> = (0..3).map(|k| hw_read(&mut nds, OAM + i * 8 + k * 2)).collect();
            vec![
                i.to_string(),
                hx(a[0] as u64, 4),
                hx(a[1] as u64, 4),
                hx(a[2] as u64, 4),
                format!("({}, {})", a[1] & 0x1FF, a[0] & 0xFF),
                "32×32".into(),
                (a[1] >> 12 & 1).to_string(),
                (a[2] >> 10 & 3).to_string(),
            ]
        })
        .collect();
    doc.table(
        &[
            ("#", R),
            ("attr0", R),
            ("attr1", R),
            ("attr2", R),
            ("pos", L),
            ("size", L),
            ("H", R),
            ("prio", R),
        ],
        &oam,
    );

    // ---- checks -------------------------------------------------------------
    let mut ck = Checks::new();
    let mut bad0 = 0;
    let mut bad1 = 0;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let e = bg0_entry(x / 8, y / 8);
            if bg0[y * WIDTH + x]
                != ref_pixel(&pal, e, &tile4((e & 0x3FF) as usize), x, y, false).unwrap_or(BACKDROP)
            {
                bad0 += 1;
            }
            let (sx, sy) = ((x + BG1_HOFS) % 256, (y + BG1_VOFS) % 256);
            let e = bg1_entry(sx / 8, sy / 8);
            if bg1[y * WIDTH + x]
                != ref_pixel(&pal, e, &tile8((e & 0x3FF) as usize), sx, sy, true)
                    .unwrap_or(BACKDROP)
            {
                bad1 += 1;
            }
        }
    }
    ck.eq(
        "BG0 4bpp text layer",
        "all 49152 pixels = GBATEK decode (tiles, H/V flip, palette bank, colour 0 transparent)",
        0,
        bad0,
    );
    ck.eq("BG1 8bpp text layer", "all pixels = decode with HOFS=4, VOFS=2", 0, bad1);
    let px = |v: &Vec<u16>, x: usize, y: usize| v[y * WIDTH + x];
    ck.hex(
        "OBJ over BG",
        "sprite 0 (prio 0) centre pixel is sprite colour 2",
        0x03FFu16,
        px(&all, 176 + 20, 96 + 20),
    );
    ck.hex("backdrop", "no layer → palette[0]", BACKDROP, px(&obj, 5, 180));
    let digit_px = (0..WIDTH * HEIGHT)
        .find(|&i| bg0[i] != BACKDROP && bg0[i] != 0x5294 && all[i] != obj[i])
        .map(|i| (bg0[i], all[i]));
    ck.ok(
        "BG0 over BG1",
        "opaque BG0 (prio 0) pixels hide BG1 (prio 1)",
        digit_px.is_some_and(|(a, b)| a == b),
        format!("{digit_px:?}"),
    );
    let transparent = (0..WIDTH * HEIGHT)
        .filter(|&i| bg0[i] == BACKDROP && obj[i] == BACKDROP)
        .all(|i| all[i] == bg1[i]);
    ck.ok(
        "transparency",
        "where BG0 texel = 0, BG1 shows through",
        transparent,
        "checked every such pixel",
    );
    doc.h2("Spec checks");
    ck.write(&mut doc);
    doc.save();
    ck.finish();
}

fn hw_read(nds: &mut NDS, addr: u32) -> u16 {
    nds.hw_mut().arm9_read::<u16>(addr)
}

// ---------------------------------------------------------------------------
// Real ROM snapshot
// ---------------------------------------------------------------------------

#[test]
fn real_rom_snapshot() {
    let rom = rom::test_rom();
    let mut nds = boot(rom, "2d_rom_snapshot");
    let budget = frame_budget(600);
    // Run until a screen shows a "rich" picture (≥ 24 distinct colours, i.e.
    // past logos and fades), or the budget runs out.
    let rich = |n: &NDS| {
        n.get_screens().iter().any(|s| {
            let mut seen = std::collections::HashSet::new();
            s.iter().any(|&p| {
                seen.insert(p & 0x7FFF);
                seen.len() >= 24
            })
        })
    };
    let (mut frames, found) = run_until(&mut nds, budget, rich);
    if found {
        // Let fades finish.
        frames += run_until(&mut nds, 90, |_| false).0;
    }
    let mut doc =
        Doc::new("hw/gpu/engine2d_rom.md", "2D engine: real ROM snapshot", "hw/gpu/engine2d.rs");
    doc.source(&format!("{} — stopped after {frames} frames (first frame with ≥ 24 distinct colours + 90 frames to let fades finish; search budget {budget})", rom.label()));
    let [top, bottom] = nds.get_screens();
    let mut both = Canvas::new(WIDTH * 2 + 4, HEIGHT, WHITE);
    both.blit_bgr555(0, 0, top, WIDTH, HEIGHT, 1);
    both.blit_bgr555(WIDTH + 4, 0, bottom, WIDTH, HEIGHT, 1);
    both.save("hw/gpu/engine2d_rom/screens.png");
    doc.image("screens", "engine2d_rom/screens.png");
    for (engine, base, name) in [(Engine::A, 0x0400_0000u32, "A"), (Engine::B, 0x0400_1000, "B")] {
        let hw = nds.hw_mut();
        let dispcnt = hw.arm9_read::<u32>(base);
        doc.h2(&format!("Engine {name}: DISPCNT = {}", hx(dispcnt as u64, 8)));
        doc.table(
            &[("Field", L), ("Bits", L), ("Value", L)],
            &[
                vec!["BG mode".into(), "0-2".into(), (dispcnt & 7).to_string()],
                vec!["BG0 = 3D".into(), "3".into(), (dispcnt >> 3 & 1).to_string()],
                vec!["OBJ 1D mapping".into(), "4".into(), (dispcnt >> 4 & 1).to_string()],
                vec![
                    "BG0..3 / OBJ enabled".into(),
                    "8-12".into(),
                    format!("{:05b}", dispcnt >> 8 & 0x1F),
                ],
                vec!["windows".into(), "13-15".into(), format!("{:03b}", dispcnt >> 13 & 7)],
                vec!["display mode".into(), "16-17".into(), (dispcnt >> 16 & 3).to_string()],
            ],
        );
        let mut rows = Vec::new();
        for bg in 0..4usize {
            let cnt = hw.arm9_read::<u16>(base + 8 + bg as u32 * 2);
            let on = dispcnt >> (8 + bg) & 1 != 0;
            rows.push(vec![
                format!("BG{bg}"),
                on.to_string(),
                hx(cnt as u64, 4),
                (cnt & 3).to_string(),
                (cnt >> 2 & 0xF).to_string(),
                (cnt >> 8 & 0x1F).to_string(),
                if cnt & 0x80 != 0 { "8bpp" } else { "4bpp" }.into(),
                (cnt >> 14).to_string(),
            ]);
            if on && !(bg == 0 && dispcnt & 8 != 0) {
                let (px, w, h) = hw.render_map(engine, bg);
                if w > 0 && h > 0 && px.len() >= w * h {
                    let mut img = Canvas::from_bgr555(&px, w, h, 1);
                    img.grid(8, [255, 255, 255, 40]);
                    let file = format!("hw/gpu/engine2d_rom/engine{name}_bg{bg}_map.png");
                    img.save(&file);
                    doc.p(&format!(
                        "Engine {name} BG{bg} full map ({w}×{h}, `HW::render_map`, 8×8 grid):"
                    ));
                    doc.image("map", &file["hw/gpu/".len()..]);
                }
            }
        }
        doc.table(
            &[
                ("BG", L),
                ("On", L),
                ("BGnCNT", R),
                ("Prio", R),
                ("Char base", R),
                ("Screen base", R),
                ("Colours", L),
                ("Size", R),
            ],
            &rows,
        );
        let (pal, w, h) = hw.render_palettes(false, 0, 0, engine, GraphicsType::BG);
        let img = Canvas::from_bgr555(&pal, w, h, (256 / w.max(1)).max(1));
        let file = format!("hw/gpu/engine2d_rom/engine{name}_bg_palette.png");
        img.save(&file);
        doc.p(&format!("Engine {name} standard BG palette (16 banks × 16 colours):"));
        doc.image("palette", &file["hw/gpu/".len()..]);
        let (tiles, w, h) = hw.render_tiles(engine, GraphicsType::BG, false, false, false, 0, 0, 0);
        if w > 0 && h > 0 {
            let mut img = Canvas::from_bgr555(&tiles, w, h, 2);
            img.grid(16, [255, 255, 255, 50]);
            let file = format!("hw/gpu/engine2d_rom/engine{name}_bg_tiles.png");
            img.save(&file);
            doc.p(&format!(
                "Engine {name} BG character data as 4bpp tiles (palette bank 0, ×2, 8×8 grid):"
            ));
            doc.image("tiles", &file["hw/gpu/".len()..]);
        }
    }
    doc.save();
}
