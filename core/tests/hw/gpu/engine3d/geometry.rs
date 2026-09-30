//! Spec tests for `core/src/hw/gpu/engine3d/geometry.rs` — the geometry
//! command set, GXFIFO, matrix stacks, clip matrix and viewport transform.
//!
//! GBATEK:
//! - "DS 3D I/O Map": <https://problemkaputt.de/gbatek.htm#ds3diomap>
//! - "DS 3D Geometry Commands": <https://problemkaputt.de/gbatek.htm#ds3dgeometrycommands>
//! - "DS 3D Matrix Stack": <https://problemkaputt.de/gbatek.htm#ds3dmatrixstack>
//! - "DS 3D Status": <https://problemkaputt.de/gbatek.htm#ds3dstatus>

use super::*;
use crate::test_support::{
    boot::io_machine,
    gx::{Gx, ONE},
    md::{Doc, L, R, hx},
    spec::Checks,
};

/// GBATEK command list: `(id, name, params, variant)`.
const COMMANDS: &[(u8, &str, usize, Option<GeometryCommand>)] = {
    use GeometryCommand::*;
    &[
        (0x00, "NOP", 0, Some(NOP)),
        (0x10, "MTX_MODE", 1, Some(MtxMode)),
        (0x11, "MTX_PUSH", 0, Some(MtxPush)),
        (0x12, "MTX_POP", 1, Some(MtxPop)),
        (0x13, "MTX_STORE", 1, Some(MtxStore)),
        (0x14, "MTX_RESTORE", 1, Some(MtxRestore)),
        (0x15, "MTX_IDENTITY", 0, Some(MtxIdentity)),
        (0x16, "MTX_LOAD_4x4", 16, Some(MtxLoad4x4)),
        (0x17, "MTX_LOAD_4x3", 12, Some(MtxLoad4x3)),
        (0x18, "MTX_MULT_4x4", 16, Some(MtxMult4x4)),
        (0x19, "MTX_MULT_4x3", 12, Some(MtxMult4x3)),
        (0x1A, "MTX_MULT_3x3", 9, Some(MtxMult3x3)),
        (0x1B, "MTX_SCALE", 3, Some(MtxScale)),
        (0x1C, "MTX_TRANS", 3, Some(MtxTrans)),
        (0x20, "COLOR", 1, Some(Color)),
        (0x21, "NORMAL", 1, Some(Normal)),
        (0x22, "TEXCOORD", 1, Some(TexCoord)),
        (0x23, "VTX_16", 2, Some(Vtx16)),
        (0x24, "VTX_10", 1, Some(Vtx10)),
        (0x25, "VTX_XY", 1, Some(VtxXY)),
        (0x26, "VTX_XZ", 1, Some(VtxXZ)),
        (0x27, "VTX_YZ", 1, Some(VtxYZ)),
        (0x28, "VTX_DIFF", 1, Some(VtxDiff)),
        (0x29, "POLYGON_ATTR", 1, Some(PolygonAttr)),
        (0x2A, "TEXIMAGE_PARAM", 1, Some(TexImageParam)),
        (0x2B, "PLTT_BASE", 1, Some(PlttBase)),
        (0x30, "DIF_AMB", 1, Some(DifAmb)),
        (0x31, "SPE_EMI", 1, Some(SpeEmi)),
        (0x32, "LIGHT_VECTOR", 1, Some(LightVector)),
        (0x33, "LIGHT_COLOR", 1, Some(LightColor)),
        (0x34, "SHININESS", 32, Some(Shininess)),
        (0x40, "BEGIN_VTXS", 1, Some(BeginVtxs)),
        (0x41, "END_VTXS", 0, Some(EndVtxs)),
        (0x50, "SWAP_BUFFERS", 1, Some(SwapBuffers)),
        (0x60, "VIEWPORT", 1, Some(Viewport)),
        (0x70, "BOX_TEST", 3, Some(BoxTest)),
        (0x71, "POS_TEST", 2, None),
        (0x72, "VEC_TEST", 1, None),
    ]
};

fn gxstat(e: &Engine3D) -> u32 {
    (0..4).map(|b| (e.read_gxstat(b) as u32) << (8 * b)).fold(0, |a, b| a | b)
}

#[test]
fn packed_gxfifo_and_ports_are_equivalent() {
    // Same scene through 4000400h (packed) and the per-command ports.
    let mut a = io_machine("gx_packed_a");
    let mut b = io_machine("gx_packed_b");
    let tri = [(0u32, 0u32), (ONE as u32, 0), (0, ONE as u32)];
    {
        let hw = a.hw_mut();
        let mut g = Gx(hw);
        g.setup(0);
        g.poly_attr(31, 1, true, true, 0);
        g.begin(0);
        for (x, y) in tri {
            g.cmd(0x23, &[x | y << 16, 0]);
        }
        g.end();
    }
    {
        let hw = b.hw_mut();
        Gx(hw).setup(0);
        Gx(hw).poly_attr(31, 1, true, true, 0);
        // Packed: BEGIN_VTXS, VTX_16, VTX_16, VTX_16 then params in order.
        hw.arm9_write::<u32>(0x0400_0400, 0x2323_2340);
        hw.arm9_write::<u32>(0x0400_0400, 0); // BEGIN_VTXS param: triangles
        for (x, y) in tri {
            hw.arm9_write::<u32>(0x0400_0400, x | y << 16);
            hw.arm9_write::<u32>(0x0400_0400, 0);
        }
        Gx(hw).end();
    }
    let va: Vec<_> = a.hw().gpu.engine3d.vertices.iter().map(|v| v.screen_coords).collect();
    let vb: Vec<_> = b.hw().gpu.engine3d.vertices.iter().map(|v| v.screen_coords).collect();
    assert_eq!(va.len(), 3);
    assert_eq!(va, vb);
}

#[test]
fn position_stack_push_pop_and_overflow_flag() {
    let mut nds = io_machine("gx_stack");
    let hw = nds.hw_mut();
    let mut g = Gx(hw);
    g.mtx_mode(1);
    for _ in 0..31 {
        g.cmd(0x11, &[]);
    }
    assert_eq!(gxstat(&hw.gpu.engine3d) >> 8 & 0x1F, 31, "31 entries");
    assert_eq!(gxstat(&hw.gpu.engine3d) >> 15 & 1, 0);
    Gx(hw).cmd(0x11, &[]);
    assert_eq!(gxstat(&hw.gpu.engine3d) >> 15 & 1, 1, "32nd push sets the error bit");
}

#[test]
fn report() {
    let mut nds = io_machine("gx_geometry_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new(
        "hw/gpu/engine3d/geometry.md",
        "3D geometry engine",
        "hw/gpu/engine3d/geometry.rs",
    );
    doc.source("register-level test through the geometry ports (no ROM data)");
    doc.p("Games feed the geometry engine either through GXFIFO (4000400h, *packed*: one word holds up to four command ids, followed by all their parameters) or by writing parameters to the command's own port 4000400h + 4·id. Lunaris pushes both into `Engine3D::gxfifo` and executes immediately until SWAP_BUFFERS halts the engine until V-Blank.");
    doc.code(
        "text",
        "packed word:  31      24 23      16 15       8 7        0\n\
         \x20             ┌─────────┬──────────┬──────────┬──────────┐\n\
         \x20             │  cmd 3  │  cmd 2   │  cmd 1   │  cmd 0   │  then params of cmd0, cmd1, …\n\
         \x20             └─────────┴──────────┴──────────┴──────────┘\n\
         vertex:  VTX_* (object) × position matrix × projection matrix = clip (x, y, z, w)\n\
         \x20        → clip against −w ≤ x,y,z ≤ w  → screen x = (x + w)·width/2w + x1\n\
         \x20                                         screen y = (−y + w)·height/2w + y1",
    );

    let mut c = Checks::new();
    let mut rows = Vec::new();
    for &(id, name, params, variant) in COMMANDS {
        let port = 0x400 + id as u32 * 4;
        let by_byte = GeometryCommand::from_byte(id);
        let by_addr = GeometryCommand::from_addr(port);
        let n = by_byte.num_params();
        let status = match variant {
            Some(v) => {
                let port_ok = id == 0 || by_addr == v;
                c.eq(
                    &format!("{name} packed id {id:02X}h"),
                    "decodes from GXFIFO byte",
                    v,
                    by_byte,
                );
                c.eq(&format!("{name} params"), "parameter word count", params, n);
                if id != 0 {
                    if port_ok {
                        c.eq(&format!("{name} port"), "decodes from its port", v, by_addr);
                    } else {
                        c.known(
                            &format!("{name} port {port:03X}h"),
                            "decodes from its port",
                            v,
                            by_addr,
                            "missing from `GeometryCommand::from_addr`",
                        );
                    }
                }
                if port_ok { "✅" } else { "⚠️ port" }
            }
            None => {
                c.known(
                    &format!("{name} ({id:02X}h)"),
                    "implemented",
                    true,
                    by_byte != GeometryCommand::Unimplemented,
                    "not implemented (treated as unknown command)",
                );
                "⚠️ missing"
            }
        };
        rows.push(vec![
            hx(id as u64, 2),
            format!("`{name}`"),
            params.to_string(),
            hx(0x0400_0000 + port as u64, 8),
            format!("`{:?}`", by_byte),
            status.into(),
        ]);
    }
    doc.h2("Command set (GBATEK ⇄ `GeometryCommand`)");
    doc.table(
        &[("Id", R), ("Command", L), ("Params", R), ("Port", R), ("Lunaris", L), ("", L)],
        &rows,
    );

    doc.h2("GXSTAT (4000600h)");
    doc.bitfield(
        "4000600h GXSTAT",
        32,
        &[
            (31, 30, "FIFO IRQ"),
            (27, 27, "BUSY"),
            (26, 26, "EMPTY"),
            (25, 25, "<HALF"),
            (24, 16, "FIFO COUNT"),
            (15, 15, "ERR"),
            (14, 14, "MBUSY"),
            (13, 13, "PROJ"),
            (12, 8, "POSVEC LEVEL"),
            (1, 1, "BOX"),
            (0, 0, "TBUSY"),
        ],
    );
    let st = gxstat(&hw.gpu.engine3d);
    c.eq("GXSTAT idle", "FIFO empty (bit 26) and < half (bit 25)", 0b11, st >> 25 & 0b11);
    let mut g = Gx(hw);
    g.mtx_mode(1);
    g.cmd(0x11, &[]);
    g.cmd(0x11, &[]);
    c.eq(
        "pos stack level",
        "two MTX_PUSH → GXSTAT bits 8-12 = 2",
        2,
        gxstat(&hw.gpu.engine3d) >> 8 & 0x1F,
    );
    Gx(hw).cmd(0x12, &[2]);
    c.eq("MTX_POP 2", "pops two levels", 0, gxstat(&hw.gpu.engine3d) >> 8 & 0x1F);
    Gx(hw).mtx_mode(0);
    Gx(hw).cmd(0x11, &[]);
    Gx(hw).cmd(0x11, &[]);
    c.eq(
        "projection stack",
        "only 1 entry: the 2nd push sets GXSTAT bit 15",
        1,
        gxstat(&hw.gpu.engine3d) >> 15 & 1,
    );
    hw.arm9_write::<u32>(0x0400_0600, 0x8000);
    c.eq(
        "error ack",
        "writing 1 to bit 15 clears the error flag",
        0,
        gxstat(&hw.gpu.engine3d) >> 15 & 1,
    );

    // Clip matrix = position × projection.
    let mut g = Gx(hw);
    g.mtx_mode(0);
    g.load4x4([
        [2.0, 0.0, 0.0, 0.0],
        [0.0, 3.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);
    g.mtx_mode(1);
    g.load4x4([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.5, 0.25, 0.0, 1.0],
    ]);
    let clip: Vec<i32> = (0..16).map(|i| hw.arm9_read::<u32>(0x0400_0640 + i * 4) as i32).collect();
    doc.h2("Clip matrix (CLIPMTX_RESULT 4000640h..67Fh)");
    doc.p("Position `T(0.5, 0.25, 0)` × projection `S(2, 3, 1)` read back as 20.12 fixed point:");
    let grid: Vec<Vec<String>> = (0..4)
        .map(|r| (0..4).map(|k| format!("{:.3}", clip[r * 4 + k] as f64 / ONE as f64)).collect())
        .collect();
    doc.table(&[("c0", R), ("c1", R), ("c2", R), ("c3", R)], &grid);
    c.eq(
        "clip[3][0..2]",
        "translation scaled by projection = (1.0, 0.75)",
        (ONE, ONE * 3 / 4),
        (clip[12], clip[13]),
    );
    c.eq("clip[0][0], clip[1][1]", "diagonal (2, 3)", (2 * ONE, 3 * ONE), (clip[0], clip[5]));

    // Viewport mapping.
    let mut g = Gx(hw);
    g.mtx_mode(0);
    g.identity();
    g.mtx_mode(2);
    g.identity();
    g.viewport(0, 0, 255, 191);
    g.poly_attr(31, 1, true, true, 0);
    g.begin(0);
    g.vtx(-1.0, 1.0, 0.0);
    g.vtx(0.0, 0.0, 0.0);
    g.vtx(0.5, 0.5, 0.0);
    g.end();
    let v: Vec<[u32; 2]> = hw.gpu.engine3d.vertices.iter().map(|v| v.screen_coords).collect();
    doc.h2("Viewport transform (identity matrices, VIEWPORT 0,0,255,191)");
    doc.table(
        &[("Object (x, y)", L), ("Screen (x, y)", L)],
        &[
            vec!["(−1, +1)".into(), format!("{:?}", v[0])],
            vec!["(0, 0)".into(), format!("{:?}", v[1])],
            vec!["(0.5, 0.5)".into(), format!("{:?}", v[2])],
        ],
    );
    c.eq("top-left", "(−1, +1) → (0, 0): +Y points up", [0, 0], v[0]);
    c.eq("centre", "(0, 0) → (128, 96)", [128, 96], v[1]);
    c.eq("quarter", "(0.5, 0.5) → (192, 48)", [192, 48], v[2]);
    let rc = hw.arm9_read::<u32>(0x0400_0604);
    c.eq(
        "RAM_COUNT",
        "bits 0-11 polygons, 16-28 vertices",
        (1, 3),
        (rc & 0xFFF, rc >> 16 & 0x1FFF),
    );

    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
