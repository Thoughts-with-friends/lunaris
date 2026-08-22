//! A Rust reference of the shader's one decision, and what it does to real
//! cases.
//!
//! # Why this exists
//!
//! The shader in [`super::shader`] cannot be unit tested: it needs a GL
//! context, a driver and a frame. But it only decides *one* thing, and that one
//! thing decides whether xBRZ is applied at all:
//!
//! > is the DS pixel under this fragment a **uniform block**?
//!
//! If it is, the pixel came from the DS's 2D engine — the compositor built it
//! by repeating one 256x192 pixel across an `n x n` block — and the filtered
//! texture is shown. If it is not, there is 3D detail inside the block, and the
//! renderer's own texels are shown so that detail survives.
//!
//! Getting that wrong is invisible in the code and obvious on screen: too
//! strict and every sprite goes unfiltered, too loose and the 3D is replaced by
//! a blurred subsample of itself. So it is mirrored here and run against frames
//! shaped the way the renderer's output is shaped.
//!
//! It is a *reference*, not the shader. What it buys is that a change to the
//! rule cannot silently stop finding the 2D again — which is exactly what
//! happened to the version before this one.

/// The DS's own screen, the grid 2D layers live on.
const DS: (usize, usize) = (256, 192);

/// Mirrors the shader's constant.
const BLOCK_TOLERANCE: f32 = 0.02;

/// Rec. 2020 luma, as the shader weights it.
fn luma_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    const LUMA: [f32; 3] = [0.2627, 0.6780, 0.0593];
    (0..3).map(|i| (a[i] - b[i]).abs() * LUMA[i]).sum()
}

/// A frame as the OpenGL renderer leaves it: `256n x 192n` texels.
struct Frame {
    texels: Vec<[f32; 3]>,
    width: usize,
    height: usize,
    scale: usize,
}

impl Frame {
    /// A frame where every DS pixel is a uniform block — what a 2D layer looks
    /// like after the compositor has repeated it `scale` times.
    fn from_ds(scale: usize, pixel: impl Fn(usize, usize) -> [f32; 3]) -> Self {
        Self::build(scale, |x, y| pixel(x / scale, y / scale))
    }

    /// A frame with detail at the *texel* level — what 3D looks like once it
    /// has been rasterised at the internal resolution.
    fn from_texels(scale: usize, texel: impl Fn(usize, usize) -> [f32; 3]) -> Self {
        Self::build(scale, texel)
    }

    fn build(scale: usize, texel: impl Fn(usize, usize) -> [f32; 3]) -> Self {
        let (width, height) = (DS.0 * scale, DS.1 * scale);
        let texels = (0..width * height).map(|at| texel(at % width, at / width)).collect();
        Self { texels, width, height, scale }
    }

    /// The shader's `BLOCK_AT`: a texel from inside one DS pixel's block, at
    /// `(u, v)` in 0..1 across it.
    fn block_at(&self, x: usize, y: usize, u: f32, v: f32) -> [f32; 3] {
        let scale = self.scale as f32;
        let tx = (((x as f32 + u) * scale) as usize).min(self.width - 1);
        let ty = (((y as f32 + v) * scale) as usize).min(self.height - 1);
        self.texels[ty * self.width + tx]
    }
}

/// What the shader shows for a fragment in DS pixel `(x, y)`.
#[derive(Debug, PartialEq, Eq)]
enum Shows {
    /// A uniform block: 2D, so the xBRZ'd texture.
    Filtered,
    /// Detail inside the block: 3D, so the renderer's own texels.
    Renderer,
}

/// The shader's decision, in Rust.
fn decide(frame: &Frame, x: usize, y: usize) -> Shows {
    let probes = [(0.15, 0.15), (0.85, 0.15), (0.15, 0.85), (0.85, 0.85)]
        .map(|(u, v)| frame.block_at(x, y, u, v));
    let inside = [
        luma_distance(probes[0], probes[1]),
        luma_distance(probes[0], probes[2]),
        luma_distance(probes[0], probes[3]),
        luma_distance(probes[1], probes[2]),
    ]
    .into_iter()
    .fold(0.0f32, f32::max);
    if inside > BLOCK_TOLERANCE { Shows::Renderer } else { Shows::Filtered }
}

const BLACK: [f32; 3] = [0.0, 0.0, 0.0];
const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

/// A 45-degree staircase, which is what a sprite's outline is made of.
fn staircase(x: usize, y: usize) -> [f32; 3] {
    if x > y { WHITE } else { BLACK }
}

#[cfg(test)]
mod tests {
    use super::{BLACK, Frame, Shows, WHITE, decide, staircase};

    /// The whole point: a sprite has to be recognised as 2D whatever the
    /// internal resolution is, because that is what sends it through xBRZ.
    #[test]
    fn a_sprite_is_filtered_at_every_internal_resolution() {
        for scale in [1usize, 2, 3, 4, 8, 16] {
            let frame = Frame::from_ds(scale, staircase);
            // On the edge of the step, and well away from it.
            for (x, y) in [(40, 40), (41, 40), (100, 20)] {
                assert_eq!(
                    decide(&frame, x, y),
                    Shows::Filtered,
                    "({x}, {y}) at {scale}x was not seen as 2D"
                );
            }
        }
    }

    /// And 3D must not be, or the filter would replace the detail the internal
    /// resolution was raised to get with a blurred subsample of it.
    #[test]
    fn rasterised_3d_keeps_the_renderers_own_pixels() {
        // A checkerboard at the texel level: the finest detail 4x can hold.
        let frame = Frame::from_texels(4, |x, y| if (x + y) % 2 == 0 { WHITE } else { BLACK });
        assert_eq!(decide(&frame, 40, 40), Shows::Renderer);

        // A gradient that changes across a block, as a shaded polygon does.
        let frame = Frame::from_texels(4, |x, _| {
            let v = x as f32 / 64.0;
            [v, v, v]
        });
        assert_eq!(decide(&frame, 40, 40), Shows::Renderer);
    }

    /// A 2D sprite over a 3D background: each has to be told apart from the
    /// other *within one frame*, which is the case the whole split exists for.
    #[test]
    fn a_2d_sprite_over_3d_is_told_apart() {
        let frame = Frame::from_texels(4, |x, y| {
            let (ds_x, ds_y) = (x / 4, y / 4);
            if (30..60).contains(&ds_x) && (30..60).contains(&ds_y) {
                staircase(ds_x, ds_y) // the sprite: uniform per DS pixel
            } else {
                let v = ((x * 7 + y * 3) % 32) as f32 / 32.0; // the 3D behind it
                [v, v, v]
            }
        });
        assert_eq!(decide(&frame, 40, 40), Shows::Filtered, "inside the sprite");
        assert_eq!(decide(&frame, 10, 10), Shows::Renderer, "in the 3D");
    }

    /// At 1x there is one texel per DS pixel, so nothing *can* vary inside one
    /// and everything is filtered — which is right, because at 1x everything
    /// the renderer produced is at the DS's own resolution.
    #[test]
    fn everything_is_2d_at_one_times() {
        let frame = Frame::from_texels(1, |x, y| if (x + y) % 2 == 0 { WHITE } else { BLACK });
        assert_eq!(decide(&frame, 40, 40), Shows::Filtered);
    }

    /// The tolerance is not zero on purpose: the compositor blending a 2D layer
    /// over a 3D one can leave a little rounding across a block, and calling
    /// that 3D would leave the sprite in front of it unfiltered.
    #[test]
    fn a_little_rounding_across_a_block_is_still_2d() {
        let frame = Frame::from_texels(4, |x, y| {
            let (ds_x, ds_y) = (x / 4, y / 4);
            // A hair of variation inside each block, under the tolerance.
            let jitter = ((x % 4 + y % 4) as f32) * 0.002;
            let base = staircase(ds_x, ds_y)[0];
            [base + jitter, base + jitter, base + jitter]
        });
        assert_eq!(decide(&frame, 40, 40), Shows::Filtered);
    }
}
