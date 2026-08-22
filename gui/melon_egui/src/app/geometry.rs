//! Where a screen is drawn, and where a click on it lands.
//!
//! Pure geometry: nothing here touches a console, which is what makes the
//! touch mapping testable without one.

use super::*;

/// A quad covering the whole GL viewport, which egui_glow has already set to
/// the paint callback's rectangle.
pub(crate) const FULL_CLIP: [f32; 4] = [-1.0, -1.0, 2.0, 2.0];

/// The second console's window. A stable id, so the viewport survives repaints.
pub(crate) fn guest_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("melon_egui-instance-2")
}

/// Paint one screen into `rect`, rotated.
///
/// Rotation is done by permuting the texture coordinates rather than by
/// transforming the destination: `rect` is already the shape the rotated screen
/// occupies, so the only question is which corner of the picture goes where.
pub(crate) fn paint_screen(
    painter: &egui::Painter,
    texture: egui::TextureId,
    rect: Rect,
    rotation: Rotation,
) {
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    let mut mesh = egui::Mesh::with_texture(texture);
    for (pos, uv) in corners.into_iter().zip(uv_corners(rotation)) {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// Which corner of the texture each destination corner samples, in the order
/// [`paint_screen`] walks them: clockwise from the top-left.
///
/// Turning the picture `n` quarter turns clockwise means each destination corner
/// shows what sat `n` corners anticlockwise of it in the source.
pub(crate) fn uv_corners(rotation: Rotation) -> [Pos2; 4] {
    /// The whole texture's corners, clockwise from the top-left.
    const UV: [Pos2; 4] = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
    std::array::from_fn(|i| UV[(i + 4 - rotation.steps()) % 4])
}

/// Where `pos` lands on a bottom screen drawn at `rect` under `rotation`, in
/// touchscreen coordinates, or `None` when it is off the panel.
///
/// Split out from [`MelonEgui::sample_touch`] so the arithmetic — the part that
/// changes with every layout option — is testable without a window.
pub(crate) fn touch_coords(rect: Rect, pos: Pos2, rotation: Rotation) -> Option<(u16, u16)> {
    if !rect.contains(pos) {
        return None;
    }
    // Position within the drawn panel, as a fraction of it.
    let u = (pos.x - rect.left()) / rect.width();
    let v = (pos.y - rect.top()) / rect.height();
    // Undo the rotation: this is the inverse of the permutation
    // `paint_screen` applies to the texture coordinates.
    let (sx, sy) = match rotation {
        Rotation::None => (u, v),
        Rotation::Cw90 => (v, 1.0 - u),
        Rotation::Cw180 => (1.0 - u, 1.0 - v),
        Rotation::Cw270 => (1.0 - v, u),
    };
    // The touchscreen has no sub-pixel resolution, and coordinates past the
    // panel are not something the hardware can report, so the scaled position
    // is floored and clamped. `rect.contains` is inclusive of the far edge,
    // which is exactly the case the clamp catches.
    Some((
        ((sx * SCREEN_WIDTH as f32) as u16).min(SCREEN_WIDTH as u16 - 1),
        ((sy * SCREEN_HEIGHT as f32) as u16).min(SCREEN_HEIGHT as u16 - 1),
    ))
}

/// A melonDS framebuffer as an egui image.
///
/// The core hands over one `u32` per pixel as `0xAARRGGBB` — byte order BGRA in
/// memory, which is what melonDS calls the format (`GPU_Soft.cpp`, "convert to
/// 32-bit BGRA"). Alpha is whatever the compositor left there, so it is
/// discarded and the pixel forced opaque.
/// One screen's framebuffer as an egui image, post-processed by `method` at
/// `factor` on the way (see [`crate::upscale`]).
///
/// The core's pixels are BGRA in memory; the swizzle here is the software
/// renderer's counterpart to the one `gl_screen`'s shader does on the GPU.
pub(crate) fn to_image(fb: &[u32], method: upscale::Method, factor: u8) -> ColorImage {
    let rgba: Vec<u8> =
        fb.iter().flat_map(|&px| [(px >> 16) as u8, (px >> 8) as u8, px as u8, 0xFF]).collect();
    let (buf, width, height) = upscale::upscale(rgba, SCREEN_WIDTH, SCREEN_HEIGHT, method, factor);
    let pixels =
        buf.as_chunks::<4>().0.iter().map(|px| Color32::from_rgb(px[0], px[1], px[2])).collect();
    ColorImage {
        size: [width, height],
        pixels,
        source_size: egui::vec2(width as f32, height as f32),
    }
}

#[cfg(test)]
mod tests {
    use egui::{Color32, Pos2, Rect, pos2, vec2};
    use melonds::{SCREEN_HEIGHT, SCREEN_WIDTH};

    use super::{Rotation, to_image, touch_coords};

    /// A bottom screen drawn at 3x, offset so that a bug that forgets to
    /// subtract the rectangle's origin cannot pass by coincidence.
    pub(crate) fn screen_rect() -> Rect {
        Rect::from_min_size(
            pos2(40.0, 300.0),
            vec2(SCREEN_WIDTH as f32 * 3.0, SCREEN_HEIGHT as f32 * 3.0),
        )
    }

    #[test]
    pub(crate) fn touch_maps_the_panel_corners_to_the_panel_corners() {
        let rect = screen_rect();
        assert_eq!(touch_coords(rect, rect.min, Rotation::None), Some((0, 0)));
        assert_eq!(
            touch_coords(rect, rect.max, Rotation::None),
            Some((SCREEN_WIDTH as u16 - 1, SCREEN_HEIGHT as u16 - 1)),
            "the far corner is inclusive, so it must clamp inside the panel",
        );
    }

    #[test]
    pub(crate) fn touch_maps_the_panel_centre_to_the_panel_centre() {
        let rect = screen_rect();
        assert_eq!(
            touch_coords(rect, rect.center(), Rotation::None),
            Some((SCREEN_WIDTH as u16 / 2, SCREEN_HEIGHT as u16 / 2)),
        );
    }

    #[test]
    pub(crate) fn touch_scales_by_the_drawn_size_not_by_pixels() {
        // A quarter of the way across a 3x panel is a quarter of the way across
        // the touchscreen, whatever the scale.
        let rect = screen_rect();
        let pos = rect.min + vec2(rect.width() / 4.0, rect.height() / 4.0);
        assert_eq!(
            touch_coords(rect, pos, Rotation::None),
            Some((SCREEN_WIDTH as u16 / 4, SCREEN_HEIGHT as u16 / 4)),
        );
    }

    #[test]
    pub(crate) fn touch_outside_the_panel_is_not_a_touch() {
        let rect = screen_rect();
        for outside in [
            pos2(rect.left() - 1.0, rect.center().y),
            pos2(rect.center().x, rect.top() - 1.0),
            pos2(rect.right() + 1.0, rect.center().y),
            pos2(rect.center().x, rect.bottom() + 1.0),
            Pos2::ZERO,
        ] {
            assert_eq!(touch_coords(rect, outside, Rotation::None), None, "at {outside:?}");
        }
    }

    /// Rotating the picture has to rotate the touch map with it, or the stylus
    /// lands somewhere other than where the player is pointing.
    #[test]
    pub(crate) fn touch_follows_the_rotation() {
        let rect = screen_rect();
        // Turned a quarter clockwise, the panel's top-left corner shows the
        // picture's bottom-left, so touching there is touching (0, max).
        assert_eq!(
            touch_coords(rect, rect.left_top(), Rotation::Cw90),
            Some((0, SCREEN_HEIGHT as u16 - 1)),
        );
        assert_eq!(
            touch_coords(rect, rect.left_top(), Rotation::Cw180),
            Some((SCREEN_WIDTH as u16 - 1, SCREEN_HEIGHT as u16 - 1)),
        );
        assert_eq!(
            touch_coords(rect, rect.left_top(), Rotation::Cw270),
            Some((SCREEN_WIDTH as u16 - 1, 0)),
        );
        // The centre is the centre whichever way up it is.
        for rotation in Rotation::ALL {
            assert_eq!(
                touch_coords(rect, rect.center(), rotation),
                Some((SCREEN_WIDTH as u16 / 2, SCREEN_HEIGHT as u16 / 2)),
                "{rotation:?}",
            );
        }
    }

    /// The property that actually matters about rotation: whatever the painter
    /// puts on screen, the touch map has to be its exact inverse, or the stylus
    /// lands somewhere other than where the player is pointing.
    ///
    /// Checked by taking each corner of the drawn panel, reading off which
    /// corner of the *picture* the painter shows there, and confirming the touch
    /// map reports that same corner.
    #[test]
    pub(crate) fn the_touch_map_inverts_what_the_painter_draws() {
        use super::uv_corners;
        let rect = screen_rect();
        let panel = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];

        for rotation in Rotation::ALL {
            for (corner, uv) in panel.into_iter().zip(uv_corners(rotation)) {
                // The texture corner the painter samples there, in touchscreen
                // coordinates, clamped the way the touch map clamps.
                let expected = (
                    ((uv.x * SCREEN_WIDTH as f32) as u16).min(SCREEN_WIDTH as u16 - 1),
                    ((uv.y * SCREEN_HEIGHT as f32) as u16).min(SCREEN_HEIGHT as u16 - 1),
                );
                assert_eq!(
                    touch_coords(rect, corner, rotation),
                    Some(expected),
                    "{rotation:?} at {corner:?}",
                );
            }
        }
    }

    /// The core hands over `0xAARRGGBB`; a swapped red and blue channel is the
    /// classic way to get a picture that is present but wrong, and it survives
    /// every "is it black?" check.
    #[test]
    pub(crate) fn framebuffer_words_keep_their_channel_order() {
        let mut fb = vec![0u32; SCREEN_WIDTH * SCREEN_HEIGHT];
        fb[0] = 0xFF_12_34_56;
        fb[1] = 0x00_FF_00_00; // pure red, transparent: alpha must be ignored
        let image = to_image(&fb, crate::upscale::Method::None, 1);
        assert_eq!(image.size, [SCREEN_WIDTH, SCREEN_HEIGHT]);
        assert_eq!(image.pixels[0], Color32::from_rgb(0x12, 0x34, 0x56));
        assert_eq!(image.pixels[1], Color32::from_rgb(0xFF, 0, 0));
        assert_eq!(image.pixels[2], Color32::BLACK);
    }
}
