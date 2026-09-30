//! Spec tests for `core/src/hw/gpu/engine3d/rendering.rs` — the software
//! rasteriser: clear plane, depth test, translucency, alpha test, texturing.
//!
//! Scenes are built through the real geometry-command ports
//! ([`crate::test_support::gx`]); after SWAP_BUFFERS the polygon/vertex lists
//! are snapshotted and fed back into the real [`Engine3D::render`] in subsets
//! (opaque only, translucent only, alpha test off, one polygon at a time) to
//! produce the layered pictures.
//!
//! GBATEK:
//! - "DS 3D Rendering": <https://problemkaputt.de/gbatek.htm#ds3doverview>
//! - "DS 3D Toon, Edge, Fog, Alpha Blending, Anti-aliasing":
//!   <https://problemkaputt.de/gbatek.htm#ds3dtooonedgefogalphablendingantialiasing>
//! - "DS 3D Texture Formats": <https://problemkaputt.de/gbatek.htm#ds3dtextureformats>

use super::*;
use crate::{
    hw::HW,
    test_support::{
        bgr555_to_rgba,
        boot::io_machine,
        gx::{Gx, model, perspective},
        md::{Doc, L, R, hx},
        png::{self, BLACK, Canvas, GRAY, MAGENTA, WHITE},
        spec::Checks,
    },
};

const W: usize = GPU::WIDTH;
const H: usize = GPU::HEIGHT;
const S: usize = 2; // picture scale

// ---------------------------------------------------------------------------
// Snapshot / re-render plumbing
// ---------------------------------------------------------------------------

fn clone_poly(p: &Polygon) -> Polygon {
    Polygon {
        start_vert: p.start_vert,
        end_vert: p.end_vert,
        y_bounds: p.y_bounds,
        attrs: p.attrs,
        tex_params: p.tex_params,
        palette_base: p.palette_base,
        is_front: p.is_front,
        original_verts: p.original_verts.clone(),
    }
}

struct Scene {
    polys: Vec<Polygon>,
    verts: Vec<Vertex>,
}

impl Scene {
    fn take(e: &Engine3D) -> Self {
        assert!(e.polygons_submitted, "scene must end with SWAP_BUFFERS");
        Scene { polys: e.polygons.iter().map(clone_poly).collect(), verts: e.vertices.clone() }
    }
    fn is_translucent(p: &Polygon) -> bool {
        p.attrs.alpha != 0x1F
    }
    fn vert_xy(&self, p: &Polygon) -> Vec<(f32, f32)> {
        self.verts[p.start_vert..p.end_vert]
            .iter()
            .map(|v| (v.screen_coords[0] as f32, v.screen_coords[1] as f32))
            .collect()
    }
}

/// Everything `render()` leaves behind for one frame.
struct Frame {
    color: Vec<u16>,
    alpha: Vec<u8>,
    depth: Vec<u32>,
    opaque_id: Vec<u8>,
    translucent_id: Vec<Option<u8>>,
}

impl Frame {
    fn canvas(&self) -> Canvas {
        Canvas::from_bgr555(&self.color, W, H, S)
    }
}

/// Runs the real rasteriser over `polys` (a subset of the scene).
/// `alpha_test`: `None` keeps the scene's setting.
fn render(
    hw: &mut HW,
    scene: &Scene,
    pick: impl Fn(usize, &Polygon) -> bool,
    alpha_test: Option<bool>,
) -> Frame {
    let gpu = &mut hw.gpu;
    let e = &mut gpu.engine3d;
    let saved = e.disp3dcnt.alpha_test;
    if let Some(a) = alpha_test {
        e.disp3dcnt.alpha_test = a;
    }
    e.polygons = scene
        .polys
        .iter()
        .enumerate()
        .filter(|(i, p)| pick(*i, p))
        .map(|(_, p)| clone_poly(p))
        .collect();
    e.vertices = scene.verts.clone();
    e.polygons_submitted = true;
    e.render(&gpu.vram, true);
    e.disp3dcnt.alpha_test = saved;
    Frame {
        color: e.frame_buffer.iter().map(|p| p.color.as_u16()).collect(),
        alpha: e.frame_buffer.iter().map(|p| p.color.a5()).collect(),
        depth: e.frame_buffer.iter().map(|p| p.depth).collect(),
        opaque_id: e.attr_buffer.iter().map(|a| a.opaque_id).collect(),
        translucent_id: e.attr_buffer.iter().map(|a| a.translucent_id).collect(),
    }
}

/// Renders polygon `i` alone over a *transparent* clear plane (α = 0), so
/// every surviving fragment takes the plain write path and records its id,
/// colour, alpha and depth — unambiguous per-polygon coverage.
fn render_solo(hw: &mut HW, scene: &Scene, i: usize, alpha_test: bool) -> Frame {
    let saved = hw.gpu.engine3d.clear_color.a;
    hw.gpu.engine3d.clear_color.a = 0;
    let f = render(hw, scene, |j, _| j == i, Some(alpha_test));
    hw.gpu.engine3d.clear_color.a = saved;
    f
}

fn covers(f: &Frame, scene: &Scene, i: usize, px: usize) -> bool {
    f.opaque_id[px] == scene.polys[i].attrs.polygon_id && f.alpha[px] != 0
}

// ---------------------------------------------------------------------------
// Pictures
// ---------------------------------------------------------------------------

fn wireframe(scene: &Scene, under: &Frame) -> Canvas {
    let mut c = Canvas::new(W * S, H * S, BLACK);
    let mut base = under.canvas();
    base.fill_rect(0, 0, (W * S) as i64, (H * S) as i64, [0, 0, 0, 170]);
    c.blit(0, 0, &base);
    for (i, p) in scene.polys.iter().enumerate() {
        let pts: Vec<(f32, f32)> =
            scene.vert_xy(p).iter().map(|&(x, y)| (x * S as f32, y * S as f32)).collect();
        let col = png::palette(i);
        c.poly_outline(&pts, col);
        for (k, &(x, y)) in pts.iter().enumerate() {
            c.fill_rect(x as i64 - 2, y as i64 - 2, 5, 5, col);
            c.label(x as i64 + 3, y as i64 + 3, &format!("{k}"), 1, WHITE);
        }
        let (cx, cy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
        let n = pts.len() as f32;
        c.label((cx / n) as i64 - 6, (cy / n) as i64 - 4, &format!("P{i}"), 2, col);
    }
    c
}

fn depth_map(f: &Frame, clear: u32) -> (Canvas, u32, u32) {
    let drawn: Vec<u32> = f.depth.iter().copied().filter(|&d| d != clear).collect();
    let (lo, hi) =
        (drawn.iter().copied().min().unwrap_or(0), drawn.iter().copied().max().unwrap_or(1));
    let legend_h = 28;
    let mut c = Canvas::new(W * S, H * S + legend_h, BLACK);
    for y in 0..H {
        for x in 0..W {
            let d = f.depth[y * W + x];
            let col = if d == clear {
                [30, 30, 30, 255]
            } else {
                png::heat((d - lo) as f32 / (hi - lo).max(1) as f32)
            };
            c.fill_rect((x * S) as i64, (y * S) as i64, S as i64, S as i64, col);
        }
    }
    let y0 = (H * S) as i64 + 4;
    for x in 0..(W * S - 160) {
        let t = x as f32 / (W * S - 160) as f32;
        c.fill_rect(80 + x as i64, y0, 1, 10, png::heat(t));
    }
    c.text(4, y0 + 2, "NEAR", 1, WHITE);
    c.text((W * S) as i64 - 76, y0 + 2, "FAR", 1, WHITE);
    c.text(80, y0 + 14, &format!("{lo:06X}"), 1, WHITE);
    c.text((W * S) as i64 - 80 - 24, y0 + 14, &format!("{hi:06X}"), 1, WHITE);
    (c, lo, hi)
}

fn id_map(f: &Frame) -> Canvas {
    let mut c = Canvas::new(W * S, H * S, BLACK);
    let mut seen = std::collections::BTreeMap::<u8, (usize, usize, usize)>::new();
    for y in 0..H {
        for x in 0..W {
            let id = f.opaque_id[y * W + x];
            let mut col = if id == 63 { [25, 25, 25, 255] } else { png::palette(id as usize * 7) };
            if let Some(t) = f.translucent_id[y * W + x] {
                // Hatch translucent coverage over the opaque id colour.
                if (x + y) % 4 == 0 {
                    col = png::palette(t as usize * 7 + 3);
                }
            }
            c.fill_rect((x * S) as i64, (y * S) as i64, S as i64, S as i64, col);
            let e = seen.entry(id).or_insert((0, 0, 0));
            *e = (e.0 + x, e.1 + y, e.2 + 1);
        }
    }
    for (id, (sx, sy, n)) in seen {
        if id != 63 && n > 40 {
            c.label((sx / n * S) as i64, (sy / n * S) as i64, &format!("ID{id}"), 2, WHITE);
        }
    }
    c
}

/// Side-by-side labelled panels.
fn panels(items: &[(&str, &Canvas)]) -> Canvas {
    let (pw, ph) = (items[0].1.w, items[0].1.h);
    let mut c = Canvas::new(items.len() * (pw + 6), ph + 16, WHITE);
    for (i, (title, img)) in items.iter().enumerate() {
        let x = (i * (pw + 6)) as i64;
        c.blit(x, 16, img);
        c.text(x + 2, 3, title, 2, BLACK);
    }
    c
}

// ---------------------------------------------------------------------------
// Scene 1: orthographic layers (depth intersection, translucency, alpha test)
// ---------------------------------------------------------------------------

const ALPHA_REF: u8 = 12;

/// Uploads a 32×32 A5I3 texture: alpha = s (0..31), palette index = t / 4.
fn upload_a5i3_texture(hw: &mut HW) {
    hw.arm9_write::<u8>(0x0400_0240, 0x80); // VRAMCNT_A → LCDC
    hw.arm9_write::<u8>(0x0400_0244, 0x80); // VRAMCNT_E → LCDC
    for t in 0..32u32 {
        for s in (0..32u32).step_by(2) {
            let texel = |s: u32| (s << 3 | (t / 4) % 8) as u16;
            hw.arm9_write::<u16>(0x0680_0000 + t * 32 + s, texel(s) | texel(s + 1) << 8);
        }
    }
    let rainbow = [0x001F, 0x023F, 0x03FF, 0x03E0, 0x7FE0, 0x7C00, 0x7C1F, 0x7FFF];
    for (i, c) in rainbow.iter().enumerate() {
        hw.arm9_write::<u16>(0x0688_0000 + i as u32 * 2, *c);
    }
    hw.arm9_write::<u8>(0x0400_0240, 0x83); // bank A → texture slot 0
    hw.arm9_write::<u8>(0x0400_0244, 0x83); // bank E → texture palette 0-3
}

fn build_layers_scene(hw: &mut HW) -> Scene {
    upload_a5i3_texture(hw);
    let mut g = Gx(hw);
    g.setup(0x000D); // texture | alpha test | alpha blending
    g.0.arm9_write::<u8>(0x0400_0340, ALPHA_REF);
    // P0: opaque red quad, flat depth z = 0.3.
    g.poly_attr(31, 1, true, true, 0);
    g.tex_image(0);
    g.begin(1);
    g.color(31, 4, 4);
    for (x, y) in [(-0.9, 0.8), (0.3, 0.8), (0.3, -0.1), (-0.9, -0.1)] {
        g.vtx(x, y, 0.3);
    }
    g.end();
    // P1: opaque green triangle whose depth crosses P0 → intersection line.
    g.poly_attr(31, 2, true, true, 0);
    g.begin(0);
    g.color(4, 28, 4);
    g.vtx(-0.7, 0.95, -0.6);
    g.vtx(0.1, 0.95, -0.6);
    g.vtx(-0.3, -0.5, 0.95);
    g.end();
    // P2: translucent blue quad (alpha 14) in front of both.
    g.poly_attr(14, 3, true, true, 0);
    g.begin(1);
    g.color(6, 10, 31);
    for (x, y) in [(-0.25, 0.45), (0.75, 0.45), (0.75, -0.55), (-0.25, -0.55)] {
        g.vtx(x, y, -0.8);
    }
    g.end();
    // P3: textured A5I3 quad → alpha test cuts its left part.
    g.poly_attr(31, 4, true, true, 0);
    g.tex_image(2 << 20 | 2 << 23 | 6 << 26);
    g.pltt_base(0);
    g.begin(1);
    g.color(31, 31, 31);
    for (x, y, s, t) in [
        (0.3, -0.35, 0.0, 0.0),
        (0.95, -0.35, 32.0, 0.0),
        (0.95, -0.95, 32.0, 32.0),
        (0.3, -0.95, 0.0, 32.0),
    ] {
        g.texcoord(s, t);
        g.vtx(x, y, -0.2);
    }
    g.end();
    g.tex_image(0);
    // P4..: Gouraud quad strip "ribbon" (mesh) in the lower left.
    g.poly_attr(31, 5, true, true, 0);
    g.begin(3);
    for i in 0..=4 {
        let x = -0.95 + i as f64 * 0.14;
        let wave = (i as f64 * 1.3).sin() * 0.08;
        g.color(i * 7, 31 - i * 6, 20);
        g.vtx(x, -0.3 + wave, 0.0);
        g.color(31 - i * 5, i * 6, 31);
        g.vtx(x, -0.85 + wave, 0.0);
    }
    g.end();
    g.swap(0);
    Scene::take(&hw.gpu.engine3d)
}

// ---------------------------------------------------------------------------
// Scene 2: perspective cube + translucent floor mesh
// ---------------------------------------------------------------------------

fn build_cube_scene(hw: &mut HW) -> Scene {
    let mut g = Gx(hw);
    g.setup(0x0008);
    g.mtx_mode(0);
    g.load4x4(perspective(60.0, 256.0 / 192.0, 0.5, 20.0));
    g.mtx_mode(2);
    g.load4x4(model(35.0, 25.0, [0.0, 0.0, -3.2]));
    let s = 0.8;
    // Faces listed counter-clockwise as seen from outside the cube.
    let faces: [([f64; 3], [[f64; 3]; 4], [u32; 3]); 6] = [
        ([0.0, 0.0, 1.0], [[-s, -s, s], [s, -s, s], [s, s, s], [-s, s, s]], [31, 8, 8]),
        ([0.0, 0.0, -1.0], [[s, -s, -s], [-s, -s, -s], [-s, s, -s], [s, s, -s]], [8, 31, 8]),
        ([1.0, 0.0, 0.0], [[s, -s, s], [s, -s, -s], [s, s, -s], [s, s, s]], [8, 8, 31]),
        ([-1.0, 0.0, 0.0], [[-s, -s, -s], [-s, -s, s], [-s, s, s], [-s, s, -s]], [31, 31, 8]),
        ([0.0, 1.0, 0.0], [[-s, s, s], [s, s, s], [s, s, -s], [-s, s, -s]], [8, 31, 31]),
        ([0.0, -1.0, 0.0], [[-s, -s, -s], [s, -s, -s], [s, -s, s], [-s, -s, s]], [31, 8, 31]),
    ];
    for (i, (_, v, c)) in faces.iter().enumerate() {
        g.poly_attr(31, 10 + i as u32, true, false, 0);
        g.begin(1);
        g.color(c[0], c[1], c[2]);
        for p in v {
            g.vtx(p[0], p[1], p[2]);
        }
        g.end();
    }
    // Translucent floor: 4×4 quad mesh at y = -1.1.
    g.poly_attr(12, 20, true, true, 0);
    g.begin(1);
    for zi in 0..4 {
        for xi in 0..4 {
            let (x0, z0) = (-2.0 + xi as f64, -2.0 + zi as f64);
            g.color(if (xi + zi) % 2 == 0 { 28 } else { 10 }, 28, 28);
            for (dx, dz) in [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)] {
                g.vtx(x0 + dx, -1.1, z0 + dz);
            }
        }
    }
    g.end();
    g.swap(0);
    Scene::take(&hw.gpu.engine3d)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn clear_plane_fills_color_depth_and_id() {
    let mut nds = io_machine("gx_clear");
    let hw = nds.hw_mut();
    Gx(hw).setup(0);
    Gx(hw).swap(0);
    let scene = Scene::take(&hw.gpu.engine3d);
    let f = render(hw, &scene, |_, _| true, None);
    assert!(f.color.iter().all(|&c| c == 0x8000 | 0x2842));
    assert!(f.depth.iter().all(|&d| d == 0x7FFF * 0x200 + 0x1FF));
    assert!(f.opaque_id.iter().all(|&i| i == 63));
}

#[test]
fn back_faces_are_culled_when_only_front_is_enabled() {
    let mut nds = io_machine("gx_cull");
    let scene = build_cube_scene(nds.hw_mut());
    let cube: Vec<_> =
        scene.polys.iter().filter(|p| (10..16).contains(&p.attrs.polygon_id)).collect();
    assert_eq!(cube.len(), 3, "a convex cube shows exactly 3 faces from this angle");
    assert!(cube.iter().all(|p| p.is_front));
}

#[test]
fn report() {
    let mut nds = io_machine("gx_report");
    let hw = nds.hw_mut();
    let mut c = Checks::new();
    let mut doc = Doc::new(
        "hw/gpu/engine3d/rendering.md",
        "3D rendering engine (rasteriser)",
        "hw/gpu/engine3d/rendering.rs",
    );
    doc.source(
        "synthetic scenes sent through the geometry-command ports (`core/tests/support/gx.rs`)",
    );
    doc.p("`Engine3D::render` runs once per frame after SWAP_BUFFERS: it clears the frame buffer to CLEAR_COLOR / CLEAR_DEPTH, draws every opaque polygon, then (with DISP3DCNT bit 3) every translucent polygon sorted by Y, and finally applies fog. Each pixel keeps colour + alpha, a 24-bit depth and attributes (opaque polygon id, translucent polygon id, fog flag).");
    doc.code(
        "text",
        "GXFIFO ─► geometry (matrices, lighting, clipping) ─► polygon list + vertex list\n\
         \x20                                                        │ SWAP_BUFFERS\n\
         \x20                                                        ▼\n\
         render():  ① clear  ──►  ② opaque pass  ──►  ③ translucent pass (Y-sorted)  ──►  ④ fog\n\
         \x20           │ colour/α/depth/id   depth test  (<, or = with POLYGON_ATTR bit 14)\n\
         \x20           │                     alpha test  (DISP3DCNT bit 2: α ≤ ALPHA_TEST_REF → discard)\n\
         \x20           ▼                     blend       C = (Cpoly·(α+1) + Cfb·(31−α)) / 32\n\
         frame_buffer[256×192] + attr_buffer[256×192] ──► copy_line() ──► 2D engine A BG0",
    );
    doc.table(
        &[("Buffer", L), ("Per pixel", L), ("Lunaris", L)],
        &[
            vec!["colour", "RGB 6-bit + α", "`FrameBufferPixel::color: FrameBufferColor`"],
            vec![
                "depth",
                "24-bit Z (or W with SWAP_BUFFERS bit 1)",
                "`FrameBufferPixel::depth: u32`",
            ],
            vec!["attributes", "fog flag, opaque id, translucent id", "`FrameBufferAttr`"],
        ],
    );

    // ---------------- Scene 1 --------------------------------------------------
    let scene = build_layers_scene(hw);
    let clear_depth = hw.gpu.engine3d.clear_depth.depth();
    let full = render(hw, &scene, |_, _| true, None);
    let opaque = render(hw, &scene, |_, p| !Scene::is_translucent(p), None);
    let translucent = render(hw, &scene, |_, p| Scene::is_translucent(p), None);
    let singles: Vec<Frame> =
        (0..scene.polys.len()).map(|i| render_solo(hw, &scene, i, false)).collect();
    let p3_tested = render_solo(hw, &scene, 3, true);

    doc.h2("Scene 1 — layers (orthographic, identity matrices)");
    doc.p("Five primitives exercise every rasteriser stage: an opaque red quad (P0), an opaque green triangle whose depth crosses it (P1), a translucent blue quad in front (P2, α = 14), an A5I3-textured quad whose texel alpha ramps 0→31 left to right (P3, alpha test ref = 12), and a Gouraud-shaded quad strip (P4…).");
    full.canvas().save("hw/gpu/engine3d/rendering/scene1_final.png");
    doc.image("final", "rendering/scene1_final.png");

    doc.h3("Layer breakdown");
    let mut rejected = full.canvas();
    rejected.fill_rect(0, 0, (W * S) as i64, (H * S) as i64, [0, 0, 0, 160]);
    let mut n_rejected = 0;
    let mut rejected_bad = 0;
    let mut kept_bad = 0;
    for i in 0..W * H {
        let (off, on) = (covers(&singles[3], &scene, 3, i), covers(&p3_tested, &scene, 3, i));
        if off && !on {
            n_rejected += 1;
            if singles[3].alpha[i] > ALPHA_REF {
                rejected_bad += 1;
            }
            rejected.fill_rect(
                ((i % W) * S) as i64,
                ((i / W) * S) as i64,
                S as i64,
                S as i64,
                MAGENTA,
            );
        }
        if on && p3_tested.alpha[i] <= ALPHA_REF {
            kept_bad += 1;
        }
    }
    let mut clear_only = Canvas::new(W * S, H * S, BLACK);
    clear_only.fill_rect(0, 0, (W * S) as i64, (H * S) as i64, bgr555_to_rgba(0x2842));
    let panel = panels(&[
        ("1 CLEAR", &clear_only),
        ("2 OPAQUE", &opaque.canvas()),
        ("3 TRANSLUCENT", &translucent.canvas()),
        ("4 ALPHA-TEST", &rejected),
        ("5 FINAL", &full.canvas()),
    ]);
    panel.save("hw/gpu/engine3d/rendering/scene1_layers.png");
    doc.image("layers", "rendering/scene1_layers.png");
    doc.table(
        &[("Panel", L), ("How it is produced", L), ("What to look for", L)],
        &[
            vec![
                "1 CLEAR",
                "CLEAR_COLOR (4000350h) only",
                "dark blue rear plane, id 63, depth 7FFFh·200h+1FFh",
            ],
            vec![
                "2 OPAQUE",
                "`render()` with only α = 31 polygons",
                "green/red intersection line = per-pixel depth test",
            ],
            vec![
                "3 TRANSLUCENT",
                "`render()` with only α < 31 polygons",
                "P2 blended over the clear colour",
            ],
            vec![
                "4 ALPHA-TEST",
                "pixels drawn with DISP3DCNT bit 2 off but missing with it on, in **magenta**",
                "left strip of P3 where texel α ≤ 12",
            ],
            vec!["5 FINAL", "normal `render()`", "all stages combined"],
        ],
    );

    doc.h3("Wireframe, polygon ids and depth");
    wireframe(&scene, &full).save("hw/gpu/engine3d/rendering/scene1_wireframe.png");
    id_map(&full).save("hw/gpu/engine3d/rendering/scene1_ids.png");
    let (dm, lo, hi) = depth_map(&full, clear_depth);
    dm.save("hw/gpu/engine3d/rendering/scene1_depth.png");
    doc.image("wireframe", "rendering/scene1_wireframe.png");
    doc.p("Polygon outlines in screen space (`Vertex::screen_coords`), vertex order numbered. Below: `attr_buffer.opaque_id` (solid) with translucent coverage hatched, and the depth buffer as a heat map (blue = near, red = far, grey = clear plane).");
    doc.image("ids", "rendering/scene1_ids.png");
    doc.image("depth", "rendering/scene1_depth.png");
    doc.p(&format!("Depth range drawn: {} (near) … {} (far).", hx(lo as u64, 6), hx(hi as u64, 6)));
    poly_table(&mut doc, &scene);

    // Checks for scene 1 ------------------------------------------------------
    c.ok(
        "clear plane",
        "uncovered pixels = CLEAR_COLOR, depth = CLEAR_DEPTH·200h+1FFh, id 63",
        (0..W * H)
            .filter(|&i| (0..singles.len()).all(|k| !covers(&singles[k], &scene, k, i)))
            .all(|i| full.depth[i] == clear_depth && full.opaque_id[i] == 63),
        "checked every pixel no polygon covers",
    );
    let (mut both, mut wrong) = (0, 0);
    for i in 0..W * H {
        let (d0, d1) = (singles[0].depth[i], singles[1].depth[i]);
        if d0 != clear_depth && d1 != clear_depth && d0.abs_diff(d1) > 0x2000 {
            both += 1;
            let want = if d1 < d0 { 2 } else { 1 };
            if opaque.opaque_id[i] != want {
                wrong += 1;
            }
        }
    }
    c.ok(
        "depth test (P0 ∩ P1)",
        "nearer fragment (smaller depth) wins",
        both > 100 && wrong == 0,
        format!("{both} overlapping pixels, {wrong} wrong"),
    );
    let mut blend_err = 0;
    let mut blend_n = 0;
    for i in 0..W * H {
        let only = |k: usize| covers(&singles[k], &scene, k, i);
        if only(2) && only(0) && !only(1) && !only(3) {
            let (src, dst) = (singles[2].color[i], opaque.color[i]);
            let a = 14u32;
            for sh in [0, 5, 10] {
                let (s5, d5) = (((src >> sh) & 0x1F) as u32, ((dst >> sh) & 0x1F) as u32);
                let want = (s5 * (a + 1) + d5 * (31 - a)) / 32;
                if (((full.color[i] >> sh) & 0x1F) as u32).abs_diff(want) > 1 {
                    blend_err += 1;
                }
            }
            blend_n += 1;
        }
    }
    c.ok(
        "alpha blending (P2 over P0)",
        "C = (Cpoly·(α+1) + Cfb·(31−α))/32 per 5-bit channel (±1)",
        blend_n > 100 && blend_err == 0,
        format!("{blend_n} pixels, {blend_err} channel errors"),
    );
    c.ok(
        "alpha test",
        "fragments with α ≤ ALPHA_TEST_REF (12) are discarded, all others kept",
        n_rejected > 50 && rejected_bad == 0 && kept_bad == 0,
        format!(
            "{n_rejected} px rejected ({rejected_bad} with α > ref), {kept_bad} kept with α ≤ ref"
        ),
    );
    c.eq(
        "translucent id",
        "P2 pixels record translucent id 3",
        true,
        full.translucent_id.contains(&Some(3)),
    );

    // ---------------- Scene 2 --------------------------------------------------
    let scene2 = build_cube_scene(hw);
    let full2 = render(hw, &scene2, |_, _| true, None);
    doc.h2("Scene 2 — perspective cube and translucent floor mesh");
    doc.p("MTX_MODE 0 + MTX_LOAD_4x4 loads a 60° perspective projection; MTX_MODE 2 loads a rotate-then-translate model matrix. The cube's six quads only enable front faces (POLYGON_ATTR bit 7), so back-face culling leaves 3; the 4×4 floor mesh (α = 12) is drawn from both sides.");
    let wf2 = wireframe(&scene2, &full2);
    let (dm2, lo2, hi2) = depth_map(&full2, clear_depth);
    panels(&[("FINAL", &full2.canvas()), ("WIREFRAME", &wf2)])
        .save("hw/gpu/engine3d/rendering/scene2_final_wire.png");
    doc.image("final + wireframe", "rendering/scene2_final_wire.png");
    panels(&[("POLYGON IDS", &id_map(&full2)), ("DEPTH", &dm2)])
        .save("hw/gpu/engine3d/rendering/scene2_ids_depth.png");
    doc.image("ids + depth", "rendering/scene2_ids_depth.png");
    doc.p(&format!("Depth range drawn: {} … {}. Perspective makes the depth of each face vary across its surface, unlike scene 1.", hx(lo2 as u64, 6), hx(hi2 as u64, 6)));
    poly_table(&mut doc, &scene2);
    let cube = scene2.polys.iter().filter(|p| (10..16).contains(&p.attrs.polygon_id)).count();
    c.eq("back-face culling", "front-only faces of a cube: 3 of 6 survive", 3, cube);
    let floor = scene2.polys.iter().filter(|p| p.attrs.polygon_id == 20).count();
    c.ok("floor mesh", "both-sided quads are never culled; only quads fully outside the view volume are clipped away", (12..=16).contains(&floor), format!("{floor} of 16 floor quads reach the polygon list"));

    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}

fn poly_table(doc: &mut Doc, scene: &Scene) {
    let rows: Vec<Vec<String>> = scene
        .polys
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let verts: Vec<String> = scene.verts[p.start_vert..p.end_vert]
                .iter()
                .map(|v| {
                    format!(
                        "({},{},{})",
                        v.screen_coords[0],
                        v.screen_coords[1],
                        hx(v.z_depth as u64, 6)
                    )
                })
                .collect();
            let mode = match p.attrs.mode {
                PolygonMode::Modulation => "modulate",
                PolygonMode::Decal => "decal",
                PolygonMode::ToonHighlight => "toon",
                PolygonMode::Shadow => "shadow",
            };
            vec![
                format!("P{i}"),
                p.attrs.polygon_id.to_string(),
                p.attrs.alpha.to_string(),
                mode.into(),
                format!(
                    "{}{}",
                    if p.attrs.render_front { "F" } else { "" },
                    if p.attrs.render_back { "B" } else { "" }
                ),
                if p.is_front { "front" } else { "back" }.into(),
                format!("{}..{}", p.y_bounds.0, p.y_bounds.1),
                (p.tex_params.format as u8).to_string(),
                verts.join(" "),
            ]
        })
        .collect();
    doc.table(
        &[
            ("#", L),
            ("id", R),
            ("α", R),
            ("mode", L),
            ("show", L),
            ("facing", L),
            ("y range", L),
            ("tex fmt", R),
            ("vertices (x, y, z_depth)", L),
        ],
        &rows,
    );
    let _ = GRAY;
}

// ---------------------------------------------------------------------------
// Real ROM capture
// ---------------------------------------------------------------------------

/// Boots the test ROM, waits for a frame with 3D polygons, and stops the
/// emulator *between* SWAP_BUFFERS and the render that would drain them.
#[test]
fn real_rom_capture() {
    use crate::test_support::{
        boot::{boot, frame_budget},
        rom,
    };
    let rom = rom::test_rom();
    let mut nds = boot(rom, "gx_rom_capture");
    let budget = frame_budget(900);
    // Skip boot logos first so we capture gameplay/title 3D, not a splash.
    let mut frames = 0;
    let mut found = false;
    while frames < budget {
        if nds.run_until_hw(1, |hw| {
            hw.gpu.engine3d.polygons_submitted && hw.gpu.engine3d.polygons.len() >= 8
        }) {
            found = true;
            break;
        }
        frames += 1;
    }
    let mut doc = Doc::new(
        "hw/gpu/engine3d/rendering_rom.md",
        "3D rendering: real ROM capture",
        "hw/gpu/engine3d/rendering.rs",
    );
    doc.source(&format!("{} — stopped at frame ≈{frames} right after SWAP_BUFFERS", rom.label()));
    if !found {
        doc.p(&format!("No frame with ≥ 8 polygons within {budget} frames (set `LUNARIS_TEST_FRAMES` to search longer)."));
        doc.save();
        return;
    }
    let hw = nds.hw_mut();
    let scene = Scene::take(&hw.gpu.engine3d);
    let clear_depth = hw.gpu.engine3d.clear_depth.depth();
    let full = render(hw, &scene, |_, _| true, None);
    let opaque = render(hw, &scene, |_, p| !Scene::is_translucent(p), None);
    let translucent = render(hw, &scene, |_, p| Scene::is_translucent(p), None);
    let (dm, lo, hi) = depth_map(&full, clear_depth);
    let wf = wireframe(&scene, &full);
    panels(&[("FINAL", &full.canvas()), ("WIREFRAME", &wf)])
        .save("hw/gpu/engine3d/rendering_rom/final_wire.png");
    panels(&[("OPAQUE", &opaque.canvas()), ("TRANSLUCENT", &translucent.canvas())])
        .save("hw/gpu/engine3d/rendering_rom/passes.png");
    panels(&[("POLYGON IDS", &id_map(&full)), ("DEPTH", &dm)])
        .save("hw/gpu/engine3d/rendering_rom/ids_depth.png");
    let d = &hw.gpu.engine3d.disp3dcnt;
    doc.table(
        &[("Item", L), ("Value", R)],
        &[
            vec!["polygons".into(), scene.polys.len().to_string()],
            vec!["vertices".into(), scene.verts.len().to_string()],
            vec![
                "translucent polygons".into(),
                scene.polys.iter().filter(|p| Scene::is_translucent(p)).count().to_string(),
            ],
            vec![
                "textured polygons".into(),
                scene.polys.iter().filter(|p| p.tex_params.format as u8 != 0).count().to_string(),
            ],
            vec![
                "DISP3DCNT texture / alpha-test / blend / fog".into(),
                format!(
                    "{} / {} / {} / {}",
                    d.texture_mapping, d.alpha_test, d.alpha_blending, d.fog_master_enable
                ),
            ],
            vec!["depth range".into(), format!("{} … {}", hx(lo as u64, 6), hx(hi as u64, 6))],
        ],
    );
    doc.p("Final frame and its polygon outlines (each polygon a distinct colour, vertices numbered in submission order):");
    doc.image("final + wireframe", "rendering_rom/final_wire.png");
    doc.p("Opaque pass vs. translucent pass, each rendered alone by the real `render()`:");
    doc.image("passes", "rendering_rom/passes.png");
    doc.p("Opaque polygon ids (translucent coverage hatched) and the depth buffer (blue = near):");
    doc.image("ids + depth", "rendering_rom/ids_depth.png");
    doc.h2("Polygon list (first 40)");
    let head = Scene {
        polys: scene.polys.iter().take(40).map(clone_poly).collect(),
        verts: scene.verts.clone(),
    };
    poly_table(&mut doc, &head);
    doc.save();
}
