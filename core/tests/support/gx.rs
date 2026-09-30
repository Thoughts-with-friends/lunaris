//! Geometry-engine driver: issues 3D commands through the per-command I/O
//! ports (4000440h + 4·id) exactly like game code, so a test scene
//! exercises the same path as a real game.
//!
//! GBATEK "DS 3D Geometry Commands": <https://problemkaputt.de/gbatek.htm#ds3dgeometrycommands>

use crate::hw::HW;

/// 1.0 in the geometry engine's 4.12 / 20.12 fixed point.
pub const ONE: i32 = 0x1000;

/// Float → 4.12 / 20.12 fixed point.
pub fn fx(v: f64) -> i32 {
    (v * ONE as f64).round() as i32
}

pub struct Gx<'a>(pub &'a mut HW);

impl Gx<'_> {
    /// Sends command `id` with its parameter words (a parameter-less command
    /// still needs one dummy write to its port).
    pub fn cmd(&mut self, id: u8, params: &[u32]) {
        let port = 0x0400_0400 + id as u32 * 4;
        if params.is_empty() {
            self.0.arm9_write::<u32>(port, 0);
        }
        for &p in params {
            self.0.arm9_write::<u32>(port, p);
        }
    }

    /// Powers the 3D engine and routes it to engine A's BG0 on the top
    /// screen, with the given DISP3DCNT value.
    pub fn setup(&mut self, disp3dcnt: u16) {
        self.0.arm9_write::<u32>(0x0400_0304, 0x820F); // POWCNT1: LCD, 2D A/B, 3D render+geometry, A on top
        self.0.arm9_write::<u32>(0x0400_0000, 0x0001_0108); // DISPCNT: graphics mode, BG0 on, BG0 = 3D
        self.0.arm9_write::<u16>(0x0400_0060, disp3dcnt);
        self.0.arm9_write::<u32>(0x0400_0350, 0x3F1F_2842); // CLEAR_COLOR: dark blue, alpha 31, id 63
        self.0.arm9_write::<u16>(0x0400_0354, 0x7FFF); // CLEAR_DEPTH: far
        self.viewport(0, 0, 255, 191);
        self.mtx_mode(0);
        self.identity();
        self.mtx_mode(2);
        self.identity();
    }

    pub fn mtx_mode(&mut self, mode: u32) {
        self.cmd(0x10, &[mode]);
    }

    pub fn identity(&mut self) {
        self.cmd(0x15, &[]);
    }

    /// MTX_LOAD_4x4 from a row-major matrix (row-vector convention: v·M).
    pub fn load4x4(&mut self, m: [[f64; 4]; 4]) {
        let p: Vec<u32> = m.iter().flatten().map(|&v| fx(v) as u32).collect();
        self.cmd(0x16, &p);
    }

    pub fn viewport(&mut self, x1: u8, y1: u8, x2: u8, y2: u8) {
        self.cmd(0x60, &[x1 as u32 | (y1 as u32) << 8 | (x2 as u32) << 16 | (y2 as u32) << 24]);
    }

    /// POLYGON_ATTR: `alpha` 0..31, `id` 0..63, `front`/`back` visibility.
    pub fn poly_attr(&mut self, alpha: u32, id: u32, front: bool, back: bool, extra: u32) {
        self.cmd(
            0x29,
            &[(back as u32) << 6 | (front as u32) << 7 | alpha << 16 | id << 24 | extra],
        );
    }

    pub fn tex_image(&mut self, param: u32) {
        self.cmd(0x2A, &[param]);
    }

    pub fn pltt_base(&mut self, base: u32) {
        self.cmd(0x2B, &[base]);
    }

    /// COLOR from 5-bit components.
    pub fn color(&mut self, r: u32, g: u32, b: u32) {
        self.cmd(0x20, &[r | g << 5 | b << 10]);
    }

    /// TEXCOORD in texels.
    pub fn texcoord(&mut self, s: f64, t: f64) {
        let (s, t) = ((s * 16.0) as i32 as u32 & 0xFFFF, (t * 16.0) as i32 as u32 & 0xFFFF);
        self.cmd(0x22, &[s | t << 16]);
    }

    /// VTX_16 with float coordinates (4.12).
    pub fn vtx(&mut self, x: f64, y: f64, z: f64) {
        let (x, y, z) = (fx(x) as u32 & 0xFFFF, fx(y) as u32 & 0xFFFF, fx(z) as u32 & 0xFFFF);
        self.cmd(0x23, &[x | y << 16, z]);
    }

    /// BEGIN_VTXS: 0 triangles, 1 quads, 2 triangle strip, 3 quad strip.
    pub fn begin(&mut self, prim: u32) {
        self.cmd(0x40, &[prim]);
    }

    pub fn end(&mut self) {
        self.cmd(0x41, &[]);
    }

    /// SWAP_BUFFERS: bit 0 manual translucent sort, bit 1 W-buffering.
    pub fn swap(&mut self, param: u32) {
        self.cmd(0x50, &[param]);
    }
}

/// Row-vector perspective projection (OpenGL-style clip space, camera
/// looking down −Z).
pub fn perspective(fov_y_deg: f64, aspect: f64, near: f64, far: f64) -> [[f64; 4]; 4] {
    let f = 1.0 / (fov_y_deg.to_radians() / 2.0).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, (far + near) / (near - far), -1.0],
        [0.0, 0.0, 2.0 * far * near / (near - far), 0.0],
    ]
}

/// Row-vector model matrix: rotate about Y then X, then translate.
pub fn model(rot_y_deg: f64, rot_x_deg: f64, t: [f64; 3]) -> [[f64; 4]; 4] {
    let (sy, cy) = rot_y_deg.to_radians().sin_cos();
    let (sx, cx) = rot_x_deg.to_radians().sin_cos();
    let ry = [[cy, 0.0, -sy], [0.0, 1.0, 0.0], [sy, 0.0, cy]];
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, sx], [0.0, -sx, cx]];
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| ry[i][k] * rx[k][j]).sum();
        }
    }
    [
        [r[0][0], r[0][1], r[0][2], 0.0],
        [r[1][0], r[1][1], r[1][2], 0.0],
        [r[2][0], r[2][1], r[2][2], 0.0],
        [t[0], t[1], t[2], 1.0],
    ]
}
