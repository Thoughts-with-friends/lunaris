//! Screen pixel conversion and placement math shared by both front ends.
//!
//! See `docs/design/egui-migration-design.md` §5.2 and §7.2. Intentionally
//! has no dependency on any GUI backend crate (egui/imgui) — it only deals
//! in plain bytes and rectangles, so both front ends can share it.

/// Native pixel dimensions of a single NDS LCD.
///
/// GBATEK "DS Video": <https://problemkaputt.de/gbatek.htm#dsvideo>
pub const SCREEN_WIDTH: usize = nds_core::nds::WIDTH;
pub const SCREEN_HEIGHT: usize = nds_core::nds::HEIGHT;

/// Converts one NDS screen buffer (15-bit BGR + 1-bit alpha, as produced by
/// [`nds_core::nds::NDS::get_screens`]) into interleaved RGBA8 bytes.
///
/// GBATEK "LCD Color Palettes" (5 bits per channel, bit 15 unused/alpha
/// here): <https://problemkaputt.de/gbatek.htm#lcdcolorpalettes>
///
/// Each 5-bit channel is expanded to 8 bits by bit replication
/// (`(c << 3) | (c >> 2)`) so pure white (0x1F) becomes 0xFF, not 0xF8.
pub fn abgr1555_to_rgba8(pixels: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() * 4);
    for &p in pixels {
        let r5 = p & 0x1F;
        let g5 = (p >> 5) & 0x1F;
        let b5 = (p >> 10) & 0x1F;
        let a = if p & 0x8000 != 0 { 0xFF } else { 0x00 };
        let expand = |c: u16| -> u8 { ((c << 3) | (c >> 2)) as u8 };
        out.push(expand(r5));
        out.push(expand(g5));
        out.push(expand(b5));
        out.push(a);
    }
    out
}

/// Arrangement of the two LCDs relative to each other.
///
/// See `docs/design/egui-migration-design.md` §7.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenLayout {
    /// Top screen above bottom screen (today's default look).
    #[default]
    Vertical,
    /// Top screen left, bottom screen right ("Horizon" mode).
    Horizontal,
}

/// An axis-aligned placement rectangle in window-local pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacementRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Computes where the top and bottom screens should be drawn inside an
/// `avail_width` x `avail_height` area, returning `(top_rect, bottom_rect)`.
///
/// - `gap`: pixels of padding between the two screens.
/// - `integer_scaling`: when true, the scale factor is floored to the
///   largest integer >= 1 so pixels land on exact multiples of the native
///   256x192 size; when false, screens fill the available area preserving
///   aspect ratio.
///
/// See `docs/design/egui-migration-design.md` §7.2.
pub fn layout_screens(
    avail_width: f32,
    avail_height: f32,
    layout: ScreenLayout,
    gap: f32,
    integer_scaling: bool,
) -> (PlacementRect, PlacementRect) {
    let (native_w, native_h) = (SCREEN_WIDTH as f32, SCREEN_HEIGHT as f32);
    let (composite_w, composite_h) = match layout {
        ScreenLayout::Vertical => (native_w, native_h * 2.0 + gap),
        ScreenLayout::Horizontal => (native_w * 2.0 + gap, native_h),
    };

    let scale = if integer_scaling {
        (avail_width / composite_w).min(avail_height / composite_h).floor().max(1.0)
    } else {
        (avail_width / composite_w).min(avail_height / composite_h)
    };

    let scaled_w = composite_w * scale;
    let scaled_h = composite_h * scale;
    let origin_x = (avail_width - scaled_w) / 2.0;
    let origin_y = (avail_height - scaled_h) / 2.0;

    let screen_w = native_w * scale;
    let screen_h = native_h * scale;
    let gap_scaled = gap * scale;

    match layout {
        ScreenLayout::Vertical => {
            let top = PlacementRect { x: origin_x, y: origin_y, width: screen_w, height: screen_h };
            let bottom = PlacementRect {
                x: origin_x,
                y: origin_y + screen_h + gap_scaled,
                width: screen_w,
                height: screen_h,
            };
            (top, bottom)
        }
        ScreenLayout::Horizontal => {
            let top = PlacementRect { x: origin_x, y: origin_y, width: screen_w, height: screen_h };
            let bottom = PlacementRect {
                x: origin_x + screen_w + gap_scaled,
                y: origin_y,
                width: screen_w,
                height: screen_h,
            };
            (top, bottom)
        }
    }
}

/// Maps a point in window-local pixel coordinates to native bottom-screen
/// coordinates (0..256, 0..192), clamped to that range. Returns `None` if
/// the point falls outside `bottom_rect`.
///
/// Used for stylus/touch input. See
/// `docs/design/egui-migration-design.md` §8.5 and GBATEK "DS Touch Screen
/// Controller (TSC)": <https://problemkaputt.de/gbatek.htm#dstouchscreencontrollertsc>
pub fn point_to_touch_coords(
    point_x: f32,
    point_y: f32,
    bottom_rect: PlacementRect,
) -> Option<(usize, usize)> {
    if point_x < bottom_rect.x
        || point_y < bottom_rect.y
        || point_x > bottom_rect.x + bottom_rect.width
        || point_y > bottom_rect.y + bottom_rect.height
        || bottom_rect.width <= 0.0
        || bottom_rect.height <= 0.0
    {
        return None;
    }

    let rel_x = (point_x - bottom_rect.x) / bottom_rect.width;
    let rel_y = (point_y - bottom_rect.y) / bottom_rect.height;
    let x = (rel_x * SCREEN_WIDTH as f32).clamp(0.0, (SCREEN_WIDTH - 1) as f32) as usize;
    let y = (rel_y * SCREEN_HEIGHT as f32).clamp(0.0, (SCREEN_HEIGHT - 1) as f32) as usize;
    Some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_white_expands_to_0xff() {
        // Bit 15 set (alpha), all 5-bit channels maxed.
        let pixel = 0x8000 | 0x1F | (0x1F << 5) | (0x1F << 10);
        let rgba = abgr1555_to_rgba8(&[pixel]);
        assert_eq!(rgba, vec![0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn transparent_black_has_zero_alpha() {
        let rgba = abgr1555_to_rgba8(&[0x0000]);
        assert_eq!(rgba, vec![0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn vertical_layout_stacks_screens() {
        let (top, bottom) = layout_screens(256.0, 384.0, ScreenLayout::Vertical, 0.0, false);
        assert_eq!(top.width, 256.0);
        assert_eq!(top.height, 192.0);
        assert_eq!(bottom.y, top.y + 192.0);
        assert_eq!(bottom.x, top.x);
    }

    #[test]
    fn horizontal_layout_places_screens_side_by_side() {
        let (top, bottom) = layout_screens(512.0, 192.0, ScreenLayout::Horizontal, 0.0, false);
        assert_eq!(bottom.x, top.x + top.width);
        assert_eq!(bottom.y, top.y);
    }

    #[test]
    fn integer_scaling_yields_exact_multiples() {
        let (top, _) = layout_screens(1000.0, 1000.0, ScreenLayout::Vertical, 0.0, true);
        assert_eq!(top.width % SCREEN_WIDTH as f32, 0.0);
        assert_eq!(top.height % SCREEN_HEIGHT as f32, 0.0);
    }

    #[test]
    fn touch_point_maps_and_clamps() {
        let rect = PlacementRect { x: 0.0, y: 0.0, width: 256.0, height: 192.0 };
        assert_eq!(point_to_touch_coords(0.0, 0.0, rect), Some((0, 0)));
        assert_eq!(point_to_touch_coords(255.0, 191.0, rect), Some((255, 191)));
        assert_eq!(point_to_touch_coords(-10.0, 0.0, rect), None);
    }
}
