//! Matrix math (20.12 fixed point) and the matrix commands
//! (MTX_LOAD / MTX_MULT / MTX_SCALE / MTX_TRANS).
//! See GBATEK "DS 3D Matrix Load/Multiply".
//!
//! ```text
//!   4x4 matrix, row-major (m[row*4 + col]), entries 20.12 fixed point (1000h = 1.0)
//!   a vertex is a row vector multiplied from the left:
//!
//!                   | m0   m1   m2   m3  |
//!     [x y z 1]  *  | m4   m5   m6   m7  |   =  [x' y' z' w']
//!                   | m8   m9   m10  m11 |
//!                   | m12  m13  m14  m15 |   <- translation (MTX_TRANS)
//!
//!   MTX_MULT_*:  current = given * current
//!     the transform given last is applied to the vertex first, so
//!     "translate, then rotate, then draw" rotates the model in place
//!     and then moves it.
//!
//!   MTX_SCALE (x, y, z)        MTX_TRANS (x, y, z)
//!   | x 0 0 0 |                | 1 0 0 0 |
//!   | 0 y 0 0 |                | 0 1 0 0 |
//!   | 0 0 z 0 |                | 0 0 1 0 |
//!   | 0 0 0 1 |                | x y z 1 |
//! ```
//!
//! GBATEK: [DS 3D Matrix Load/Multiply](https://problemkaputt.de/gbatek.htm#ds3dmatrixloadmultiply),
//! [DS 3D Matrix Examples (Maths Basics)](https://problemkaputt.de/gbatek.htm#ds3dmatrixexamplesmathsbasics)
use super::mul;
use crate::engine_3d::state::{Gpu3D, Mtx};

/// `m = s * m` (4x4).
pub(super) fn mtx_mult_4x4(m: &mut Mtx, s: &[i32]) {
    let t = *m;
    for r in 0..4 {
        for c in 0..4 {
            m[r * 4 + c] =
                ((0..4).map(|k| mul(s[r * 4 + k], t[k * 4 + c])).sum::<i64>() >> 12) as i32;
        }
    }
}

/// `m = s * m` with a 4x3 `s` (implicit last column 0,0,0,1).
pub(super) fn mtx_mult_4x3(m: &mut Mtx, s: &[i32]) {
    let t = *m;
    for r in 0..4 {
        for c in 0..4 {
            let mut v: i64 = (0..3).map(|k| mul(s[r * 3 + k], t[k * 4 + c])).sum();
            if r == 3 {
                // DS 3D Matrix Load/Multiply: https://problemkaputt.de/gbatek.htm#ds3dmatrixloadmultiply
                v += mul(0x1000, t[12 + c]); // 1.0 in 20.12 fixed point
            }
            m[r * 4 + c] = (v >> 12) as i32;
        }
    }
}

/// `m = s * m` with a 3x3 `s` (only the upper 3 rows change).
pub(super) fn mtx_mult_3x3(m: &mut Mtx, s: &[i32]) {
    let t = *m;
    for r in 0..3 {
        for c in 0..4 {
            m[r * 4 + c] =
                ((0..3).map(|k| mul(s[r * 3 + k], t[k * 4 + c])).sum::<i64>() >> 12) as i32;
        }
    }
}

/// MTX_SCALE: scales the first three rows by x, y, z.
pub(super) fn mtx_scale(m: &mut Mtx, s: &[i32]) {
    for r in 0..3 {
        for c in 0..4 {
            m[r * 4 + c] = (mul(s[r], m[r * 4 + c]) >> 12) as i32;
        }
    }
}

/// MTX_TRANS: `row3 += x*row0 + y*row1 + z*row2`.
pub(super) fn mtx_translate(m: &mut Mtx, s: &[i32]) {
    for c in 0..4 {
        let v = mul(s[0], m[c]) + mul(s[1], m[4 + c]) + mul(s[2], m[8 + c]);
        m[12 + c] = m[12 + c].wrapping_add((v >> 12) as i32);
    }
}

/// MTX_LOAD_4x3: loads a 4x3 matrix (last column 0,0,0,1).
pub(super) fn mtx_load_4x3(m: &mut Mtx, s: &[i32]) {
    *m = [
        s[0], s[1], s[2], 0, //
        s[3], s[4], s[5], 0, //
        s[6], s[7], s[8], 0, //
        s[9], s[10], s[11], 0x1000, // 1.0 in 20.12 fixed point
    ];
}

/// Transforms `(x, y, z, 1)` by `m`.
pub(super) fn transform(v: [i32; 3], m: &Mtx) -> [i32; 4] {
    let v = [
        i64::from(v[0]),
        i64::from(v[1]),
        i64::from(v[2]),
        0x1000_i64, // w = 1.0 in 20.12 fixed point
    ];
    core::array::from_fn(|c| {
        ((v[0] * i64::from(m[c])
            + v[1] * i64::from(m[4 + c])
            + v[2] * i64::from(m[8 + c])
            + v[3] * i64::from(m[12 + c]))
            >> 12) as i32
    })
}

impl Gpu3D {
    /// Matrix load/multiply commands 0x16-0x1C.
    pub(super) fn matrix_command(&mut self, cmd: u8, p: &[i32; 32]) {
        let apply = |m: &mut Mtx| match cmd {
            0x16 => m.copy_from_slice(&p[..16]), // MTX_LOAD_4x4
            0x17 => mtx_load_4x3(m, p),          // MTX_LOAD_4x3
            0x18 => mtx_mult_4x4(m, p),          // MTX_MULT_4x4
            0x19 => mtx_mult_4x3(m, p),          // MTX_MULT_4x3
            0x1A => mtx_mult_3x3(m, p),          // MTX_MULT_3x3
            0x1B => mtx_scale(m, p),             // MTX_SCALE
            _ => mtx_translate(m, p),
        };
        match self.matrix_mode {
            0 => {
                apply(&mut self.proj_matrix);
                self.clip_matrix_dirty = true;
            }
            3 => apply(&mut self.tex_matrix),
            mode => {
                apply(&mut self.pos_matrix);
                // SCALE never touches the vector matrix.
                if mode == 2 && cmd != 0x1B {
                    // MTX_SCALE never modifies the vector matrix
                    apply(&mut self.vec_matrix);
                }
                self.clip_matrix_dirty = true;
            }
        }
    }
}
