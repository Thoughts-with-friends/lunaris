//! Edge walking and perspective-correct interpolation (melonDS `Slope` and
//! `Interpolator`): reproduce the hardware's rounding of polygon edges and
//! of colors / texture coordinates / depth across them.
//!
//! ## Edge walking
//!
//! ```text
//!   An edge from (x0, y0) to (x1, y1) is stepped one scanline at a time.
//!   X is kept in 14.18 fixed point (1.0 = 40000h); increment = |dx| / dy.
//!
//!    y-major edge (increment <= 1.0)       x-major edge (increment > 1.0)
//!    at most one pixel per row             a run of pixels per row
//!      *                                     *###
//!       *                                        ####
//!        *                                           ####
//!         *                                              ###*
//!                                          the run is the "left/right edge"
//!                                          segment of the span in band.rs
//!
//!   The hardware's rounding is reproduced exactly: x-major edges start half a
//!   pixel in (+0.5 = 20000h), and left and right edges round in opposite
//!   directions, so two polygons sharing an edge neither leave a gap nor draw
//!   the shared pixels twice.
//! ```
//!
//! ## Perspective-correct interpolation (Interp)
//!
//! ```text
//!   A color or texture coordinate between two vertices is NOT linear in
//!   screen distance; it is weighted by each vertex's w (its depth):
//!
//!       screen:   v0 |------- t -------x---------------| v1      (t = 0..1)
//!       value(t) = (v0*(1-t)/w0 + v1*t/w1) / ((1-t)/w0 + t/w1)
//!
//!   example: a floor tile running away from the camera (w0 small, w1 large).
//!   The screen midpoint shows a point closer to v0 in 3D, so checkerboard
//!   tiles shrink with distance as they should; linear interpolation would
//!   give them all the same width on screen:
//!
//!       linear (wrong)          perspective-correct
//!       +--+--+--+--+            +---+---+--+-+
//!       |##|  |##|  |            |###|   |##| |
//!       +--+--+--+--+            +---+---+--+-+
//!
//!   equal w on both ends -> plain linear interpolation ("linear" flag).
//!   Edges use 9 bits of weight precision, spans 8 bits, as the hardware does.
//!   Depth: linear for Z-buffering, perspective-correct for W-buffering.
//! ```
//!
//! GBATEK: [DS 3D Polygon Definitions by Vertices](https://problemkaputt.de/gbatek.htm#ds3dpolygondefinitionsbyvertices),
//! [DS 3D Texture Coordinates](https://problemkaputt.de/gbatek.htm#ds3dtexturecoordinates)
/// Perspective-correct interpolator along an edge (`DIR = true`, 9-bit
/// precision) or along a span (`DIR = false`, 8-bit precision).
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Interp<const DIR: bool> {
    pub(super) x0: i32,
    pub(super) xdiff: i32,
    pub(super) x: i32,
    pub(super) shift: u32,
    pub(super) linear: bool,
    pub(super) wbuffer: bool,
    pub(super) xrecip_z: i32,
    pub(super) w0n: i32,
    pub(super) w0d: i32,
    pub(super) w1d: i32,
    pub(super) yfactor: u32,
}

impl<const DIR: bool> Interp<DIR> {
    /// Interpolator between `x0` and `x1` with depths/W `w0`, `w1`.
    pub(super) fn new(x0: i32, x1: i32, w0: i32, w1: i32, wbuffer: bool) -> Self {
        let mut s = Self::default();
        s.setup(x0, x1, w0, w1, wbuffer);
        s
    }

    /// Re-initializes the interpolator (perspective weights from W).
    pub(super) fn setup(&mut self, x0: i32, x1: i32, w0: i32, w1: i32, wbuffer: bool) {
        self.x0 = x0;
        self.xdiff = x1.wrapping_sub(x0);
        self.wbuffer = wbuffer;
        self.xrecip_z = if self.xdiff != 0 {
            (1 << 22) / self.xdiff // reciprocal in 22-bit fixed point
        } else {
            0
        };
        let mask = if DIR { 0x7E } else { 0x7F }; // w fractional bits that rule out the linear (non perspective) shortcut
        self.linear = w0 == w1 && w0 & mask == 0 && w1 & mask == 0;
        if DIR {
            // Along edges, the W values are reduced to 15 bits.
            self.w0n = w0 >> 1;
            self.w0d = (w0 + ((w0 & !w1) & 1)) >> 1;
            self.w1d = w1 >> 1;
            self.shift = 9;
        } else {
            self.w0n = w0;
            self.w0d = w0;
            self.w1d = w1;
            self.shift = 8;
        }
    }

    /// Moves the interpolator to position `x`.
    pub(super) fn set_x(&mut self, x: i32) {
        let x = x - self.x0;
        self.x = x;
        if self.xdiff != 0 && (!self.linear || self.wbuffer) {
            // 64-bit: at native size these fit in 32 bits (as on hardware),
            // but edges of a scaled raster (raster::hires) are longer.
            let num = (i64::from(x) * i64::from(self.w0n)) << self.shift;
            let den = i64::from(x) * i64::from(self.w0d)
                + i64::from(self.xdiff - x) * i64::from(self.w1d);
            self.yfactor = (num as u64).checked_div(den as u64).unwrap_or(0) as u32;
        }
    }

    /// Perspective-correct interpolation of a vertex attribute.
    pub(super) fn interpolate(&self, y0: i32, y1: i32) -> i32 {
        if self.xdiff == 0 || y0 == y1 {
            return y0;
        }
        if self.linear {
            if y0 < y1 {
                y0 + (i64::from(y1 - y0) * i64::from(self.x) / i64::from(self.xdiff)) as i32
            } else {
                y1 + (i64::from(y0 - y1) * i64::from(self.xdiff - self.x) / i64::from(self.xdiff))
                    as i32
            }
        } else if y0 < y1 {
            y0.wrapping_add((((y1 - y0) as u32).wrapping_mul(self.yfactor) >> self.shift) as i32)
        } else {
            let f = (1u32 << self.shift).wrapping_sub(self.yfactor);
            y1.wrapping_add((((y0 - y1) as u32).wrapping_mul(f) >> self.shift) as i32)
        }
    }

    /// Depth interpolation (linear for Z buffering, perspective for W).
    pub(super) fn interpolate_z(&self, z0: i32, z1: i32) -> i32 {
        if self.xdiff == 0 || z0 == z1 {
            return z0;
        }
        if self.wbuffer {
            if z0 < z1 {
                z0 + ((i64::from(z1 - z0) * i64::from(self.yfactor)) >> self.shift) as i32
            } else {
                let f = (1u32 << self.shift).wrapping_sub(self.yfactor);
                z1 + ((i64::from(z0 - z1) * i64::from(f)) >> self.shift) as i32
            }
        } else {
            let (base, mut disp, factor) =
                if z0 < z1 { (z0, z1 - z0, self.x) } else { (z1, z0 - z1, self.xdiff - self.x) };
            let r = i64::from(self.xrecip_z);
            if DIR {
                let mut shift = 0;
                while disp > 0x3FF {
                    // keep the displacement within 10 bits
                    disp >>= 1;
                    shift += 1;
                }
                base.wrapping_add(
                    (((i64::from(disp) * i64::from(factor) * r) >> 22) << shift) as i32,
                )
            } else {
                disp >>= 9;
                base.wrapping_add(((i64::from(disp) * i64::from(factor) * r) >> 13) as i32)
            }
        }
    }
}

/// One polygon edge. `RIGHT` selects the right-edge rounding rules.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Slope<const RIGHT: bool> {
    pub(super) increment: i32,
    pub(super) negative: bool,
    pub(super) x_major: bool,
    pub(super) interp: Interp<true>,
    pub(super) x0: i32,
    pub(super) xmin: i32,
    pub(super) xmax: i32,
    pub(super) xlen: i32,
    pub(super) ylen: i32,
    pub(super) dx: i32,
    pub(super) y: i32,
    pub(super) xcov_incr: i32,
}

impl<const RIGHT: bool> Slope<RIGHT> {
    /// Zero-height edge at `x0` (flat polygon rows).
    pub(super) fn setup_dummy(&mut self, x0: i32, wbuffer: bool) -> i32 {
        self.dx = 0;
        self.x0 = x0;
        self.xmin = x0;
        self.xmax = x0;
        self.increment = 0;
        self.x_major = false;
        self.interp.setup(0, 0, 0, 0, wbuffer);
        self.interp.set_x(0);
        self.xcov_incr = 0;
        x0
    }

    /// Sets the edge up from two vertices at row `y`; returns the starting X.
    #[expect(clippy::too_many_arguments)]
    pub(super) fn setup(
        &mut self,
        x0: i32,
        x1: i32,
        y0: i32,
        y1: i32,
        w0: i32,
        w1: i32,
        y: i32,
        wbuffer: bool,
    ) -> i32 {
        self.x0 = x0;
        self.y = y;
        if x1 > x0 {
            self.xmin = x0;
            self.xmax = x1 - 1;
            self.negative = false;
        } else if x1 < x0 {
            self.xmin = x1;
            self.xmax = x0 - 1;
            self.negative = true;
        } else {
            self.xmin = x0;
            self.xmax = x0;
            self.negative = false;
        }
        self.xlen = self.xmax + 1 - self.xmin;
        self.ylen = y1 - y0;

        self.increment = if self.ylen == 0 {
            0
        } else if self.ylen == self.xlen && self.xlen != 1 {
            0x40000 // exactly 45 degrees: 1.0 in 14.18 fixed point
        } else {
            let yrecip = (1 << 18) / self.ylen; // reciprocal in 18-bit fixed point
            ((x1 - x0).wrapping_mul(yrecip)).wrapping_abs()
        };
        self.x_major = self.increment > 0x40000; // more than one pixel per scanline: x-major edge

        self.dx = if RIGHT {
            if self.x_major {
                if self.negative {
                    0x20000 + 0x40000 // 0.5 + 1.0 in 14.18 fixed point
                } else {
                    self.increment - 0x20000 // start half a pixel in (0.5 = 20000h)
                }
            } else if self.increment != 0 {
                if self.negative { 0x40000 } else { 0 } // 1.0 in 14.18 fixed point
            } else {
                0
            }
        } else if self.x_major {
            if self.negative {
                (self.increment - 0x20000) + 0x40000 // start half a pixel in, plus 1.0
            } else {
                0x20000 // 0.5 in 14.18 fixed point
            }
        } else if self.increment != 0 {
            if self.negative { 0x40000 } else { 0 } // 1.0 in 14.18 fixed point
        } else {
            0
        };
        self.dx = self.dx.wrapping_add((y - y0).wrapping_mul(self.increment));

        let x = self.x_val();
        let offset = i32::from(self.increment >= 0x40000 && (RIGHT ^ self.negative)); // x-major edges are offset by one scanline
        self.interp.setup(y0 - offset, y1 - offset, w0, w1, wbuffer);
        self.interp.set_x(y);
        if self.x_major {
            self.xcov_incr = (self.ylen << 10) / self.xlen; // coverage step in 10-bit fixed point
        }
        x
    }

    /// Advances the edge one row; returns the new X.
    pub(super) fn step(&mut self) -> i32 {
        self.dx = self.dx.wrapping_add(self.increment);
        self.y += 1;
        let x = self.x_val();
        self.interp.set_x(self.y);
        x
    }

    /// Current X of the edge in pixels.
    pub(super) fn x_val(&self) -> i32 {
        let ret = if self.negative { self.x0 - (self.dx >> 18) } else { self.x0 + (self.dx >> 18) };
        ret.clamp(self.xmin, self.xmax.max(self.xmin))
    }

    /// Edge length on this scanline and anti-aliasing coverage
    /// (bit 31 set: x-major, bits 12-21 start coverage, 0-9 increment).
    pub(super) fn edge_params(&self, swapped: bool) -> (i32, u32) {
        if self.x_major {
            let mut length = 1;
            if !swapped || RIGHT {
                length = if RIGHT ^ self.negative {
                    (self.dx >> 18) - (self.dx.wrapping_sub(self.increment) >> 18)
                } else {
                    (self.dx.wrapping_add(self.increment) >> 18) - (self.dx >> 18)
                };
            }
            let mut startx = self.dx >> 18;
            if self.negative {
                startx = self.xlen - startx;
            }
            if RIGHT {
                startx = startx - length + 1;
            }
            // DS 3D Toon, Edge, Fog, Alpha-Blending, Anti-Aliasing: https://problemkaputt.de/gbatek.htm#ds3dtoonedgefogalphablendingantialiasing
            let startcov = (((startx << 10) + 0x1FF) * self.ylen) / self.xlen; // starting edge coverage, 10-bit fraction
            let coverage =
                (1 << 31) | (((startcov & 0x3FF) as u32) << 12) | (self.xcov_incr & 0x3FF) as u32; // packed: bit 31 flag, start coverage (bits 12-21), coverage step (bits 0-9)
            (if swapped { 1 } else { length }, coverage)
        } else {
            let coverage = if self.increment == 0 {
                if swapped { 0 } else { 31 }
            } else {
                let mut cov = ((self.dx >> 9) + (self.increment >> 10)) >> 4;
                if (cov >> 5) != (self.dx >> 18) {
                    cov = 31;
                }
                cov &= 0x1F; // 5-bit coverage
                let flip = if swapped { RIGHT ^ self.negative } else { !(RIGHT ^ self.negative) };
                if flip {
                    cov = 0x1F - cov; // invert coverage (31 - cov)
                }
                cov as u32
            };
            (1, coverage)
        }
    }
}
