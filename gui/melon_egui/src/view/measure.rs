//! How big each screen wants to be, before anything is placed.

use super::*;

/// The aspect-ratio multiplier for one screen, resolving
/// [`AspectRatio::Window`] against the area being filled.
pub(crate) fn aspect_multiplier(aspect: AspectRatio, area: Rect) -> f32 {
    aspect.multiplier().unwrap_or_else(|| {
        // "window": as wide relative to native as the window itself is.
        let window = area.width() / area.height().max(1.0);
        (window / (4.0 / 3.0)).max(0.1)
    })
}

/// One screen's size in DS pixels, with its aspect ratio applied and the
/// rotation taken into account: a quarter turn puts it on its side.
pub(crate) fn screen_size(opts: &ViewOptions, aspect: AspectRatio, area: Rect) -> Vec2 {
    let width = SCREEN_WIDTH as f32 * aspect_multiplier(aspect, area);
    if opts.rotation.is_sideways() {
        vec2(SCREEN_HEIGHT as f32, width)
    } else {
        vec2(width, SCREEN_HEIGHT as f32)
    }
}

/// The two screens' sizes as `(top, bottom)`.
pub(crate) fn screen_sizes(opts: &ViewOptions, area: Rect) -> (Vec2, Vec2) {
    (screen_size(opts, opts.aspect_top, area), screen_size(opts, opts.aspect_bottom, area))
}

/// Whether the two screens sit beside each other rather than stacked.
///
/// `Natural` means "as the console is held": rotated onto its side, the screens
/// end up next to each other.
pub(crate) fn is_side_by_side(opts: &ViewOptions) -> bool {
    match opts.layout {
        ScreenLayout::Vertical => false,
        ScreenLayout::Horizontal => true,
        // Hybrid always puts its small screen beside the large one.
        ScreenLayout::Hybrid => true,
        ScreenLayout::Natural => opts.rotation.is_sideways(),
    }
}

/// The bounding box of everything drawn, in DS pixels at 1x.
pub(crate) fn content_size(opts: &ViewOptions, area: Rect) -> Vec2 {
    let (top, bottom) = screen_sizes(opts, area);
    let gap = opts.gap as f32;
    if opts.sizing.is_single() {
        return if opts.sizing == ScreenSizing::TopOnly { top } else { bottom };
    }
    if is_side_by_side(opts) {
        vec2(top.x + gap + bottom.x, top.y.max(bottom.y))
    } else {
        vec2(top.x.max(bottom.x), top.y + gap + bottom.y)
    }
}

/// The largest scale at which `size` fits inside `area`, honouring
/// `integer_scaling`.
pub(crate) fn fit(area: Rect, size: Vec2, opts: &ViewOptions) -> f32 {
    // An explicit scale is explicit: it is not then rounded by
    // `integer_scaling`, and it is allowed to overflow the window (the panel
    // clips, so the result is a crop rather than a squeeze).
    if let Some(scale) = opts.display_scale {
        return scale.max(0.0);
    }
    let mut scale = (area.width() / size.x).min(area.height() / size.y);
    if !scale.is_finite() || scale <= 0.0 {
        return 0.0;
    }
    if opts.integer_scaling {
        // Never below 1x: a window too small for one whole pixel per pixel is
        // better overflowing than blank.
        scale = scale.floor().max(1.0);
    }
    scale
}

/// The window size that shows both screens at exactly `scale`, plus `chrome`
/// for the menu bar.
///
/// melonDS's "Screen size" entries resize the window rather than setting a zoom,
/// so the fitted scale then works out to exactly what was asked for.
pub fn window_size_for_scale(scale: f32, opts: &ViewOptions, chrome: f32) -> Vec2 {
    // A square reference area, so an `AspectRatio::Window` screen resolves to
    // native rather than to whatever the current window happens to be.
    let reference = Rect::from_min_size(egui::Pos2::ZERO, Vec2::splat(1024.0));
    let content = content_size(opts, reference) * scale;
    vec2(content.x, content.y + chrome)
}
