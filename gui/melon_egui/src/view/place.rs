//! Where the two screens actually land in the window.

use super::*;

/// Where each screen is drawn. `None` means that screen is not shown, which
/// only [`ScreenSizing::TopOnly`] and [`ScreenSizing::BottomOnly`] cause.
#[derive(Debug, PartialEq)]
pub struct Layout {
    pub top: Option<Rect>,
    pub bottom: Option<Rect>,
}

/// Fit the screens into `area`.
///
/// `opts.sizing` is expected to be concrete -- call [`ScreenSizing::resolve`]
/// first, since `Auto` depends on emulator state this function cannot see. A
/// degenerate `area` yields empty rectangles rather than a panic.
pub fn layout(area: Rect, opts: &ViewOptions) -> Layout {
    let (top_size, bottom_size) = screen_sizes(opts, area);

    // One screen filling the area on its own.
    if opts.sizing.is_single() {
        // Swapping with one screen shown selects the other screen, matching
        // melonDS: the choice is which panel occupies the window.
        let top_shown = (opts.sizing == ScreenSizing::TopOnly) != opts.swap;
        let size = if top_shown { top_size } else { bottom_size };
        let placed = Rect::from_center_size(area.center(), size * fit(area, size, opts));
        return Layout { top: top_shown.then_some(placed), bottom: (!top_shown).then_some(placed) };
    }

    // Hybrid is a shape rather than a sizing, so it takes over from `Even`.
    if opts.layout == ScreenLayout::Hybrid {
        return hybrid(area, opts, top_size, bottom_size);
    }
    match opts.sizing {
        ScreenSizing::EmphasizeTop | ScreenSizing::EmphasizeBottom => {
            emphasized(area, opts, top_size, bottom_size)
        }
        _ => even(area, opts, top_size, bottom_size),
    }
}

/// Both screens at one shared scale, the pair centred in `area`.
pub(crate) fn even(area: Rect, opts: &ViewOptions, top_size: Vec2, bottom_size: Vec2) -> Layout {
    let content = content_size(opts, area);
    let scale = fit(area, content, opts);
    place_pair(area, opts, top_size * scale, bottom_size * scale, opts.gap as f32 * scale)
}

/// One screen as large as it will go, the other fitted into what is left.
///
/// Ported from melonDS's `ScreenLayout.cpp` (the `screenSizing_EmphTop` branch):
/// the primary screen takes the largest scale the whole area allows, and the
/// secondary gets the leftover -- unless the leftover is too small for the
/// secondary at 1x, in which case the primary shrinks to make room.
pub(crate) fn emphasized(
    area: Rect,
    opts: &ViewOptions,
    top_size: Vec2,
    bottom_size: Vec2,
) -> Layout {
    let emph_top = opts.sizing == ScreenSizing::EmphasizeTop;
    let (prim, sec) = if emph_top { (top_size, bottom_size) } else { (bottom_size, top_size) };
    let gap = opts.gap as f32;
    let sideways = is_side_by_side(opts);

    let mut prim_scale = fit(area, prim, opts);
    let mut sec_scale = 1.0;
    // The axis the two screens share out between them, and how far each may
    // stretch across the other one.
    let (available, prim_extent, sec_extent) = if sideways {
        (area.width() - gap, prim.x, sec.x)
    } else {
        (area.height() - gap, prim.y, sec.y)
    };
    let prim_across = if sideways { area.height() / prim.y } else { area.width() / prim.x };
    let sec_across = if sideways { area.height() / sec.y } else { area.width() / sec.x };

    if available - prim_extent * prim_scale < sec_extent {
        // No room for the secondary at 1x, so the primary gives some back.
        prim_scale = ((available - sec_extent) / prim_extent).min(prim_across).max(0.0);
    } else {
        sec_scale = ((available - prim_extent * prim_scale) / sec_extent).min(sec_across);
    }
    if opts.integer_scaling {
        prim_scale = prim_scale.floor().max(1.0);
        sec_scale = sec_scale.floor().max(1.0);
    }
    for scale in [&mut prim_scale, &mut sec_scale] {
        if !scale.is_finite() || *scale < 0.0 {
            *scale = 0.0;
        }
    }

    let (top_scale, bottom_scale) =
        if emph_top { (prim_scale, sec_scale) } else { (sec_scale, prim_scale) };
    place_pair(area, opts, top_size * top_scale, bottom_size * bottom_scale, gap)
}

/// One screen large with the other beside it, small.
///
/// A simplification of melonDS's hybrid, which shows a third panel: there, the
/// large view sits next to *both* screens at small size. Here the large view is
/// one screen and the small one is the other, so no screen is drawn twice. The
/// large screen is 4/3 the small one, the ratio melonDS uses
/// (`ScreenLayout.cpp`, `hybScale`).
pub(crate) fn hybrid(area: Rect, opts: &ViewOptions, top_size: Vec2, bottom_size: Vec2) -> Layout {
    /// How much bigger the large panel is than the small one.
    const HYBRID_RATIO: f32 = 4.0 / 3.0;

    // The emphasised screen is the top one, or the bottom when swapped -- the
    // same meaning `swap` has everywhere else.
    let large_is_top = !opts.swap;
    let (large, small) =
        if large_is_top { (top_size, bottom_size) } else { (bottom_size, top_size) };
    let gap = opts.gap as f32;

    // Lay it out at 1x first, then fit the whole arrangement.
    let content =
        vec2(large.x * HYBRID_RATIO + gap + small.x, (large.y * HYBRID_RATIO).max(small.y));
    let scale = fit(area, content, opts);

    let large_drawn = large * (HYBRID_RATIO * scale);
    let small_drawn = small * scale;
    let block = Rect::from_center_size(area.center(), content * scale);

    // Large on the left, each panel centred vertically against the block.
    let large_rect = Rect::from_min_size(
        pos2(block.left(), block.center().y - large_drawn.y / 2.0),
        large_drawn,
    );
    let small_rect = Rect::from_min_size(
        pos2(large_rect.right() + gap * scale, block.center().y - small_drawn.y / 2.0),
        small_drawn,
    );

    let (top, bottom) =
        if large_is_top { (large_rect, small_rect) } else { (small_rect, large_rect) };
    Layout { top: Some(top), bottom: Some(bottom) }
}

/// Place two already-scaled screens in their slots and centre the pair in
/// `area`, applying [`ViewOptions::swap`].
pub(crate) fn place_pair(
    area: Rect,
    opts: &ViewOptions,
    top: Vec2,
    bottom: Vec2,
    gap: f32,
) -> Layout {
    let sideways = is_side_by_side(opts);
    // What goes in the first slot: the bottom screen when swapped.
    let (first, second) = if opts.swap { (bottom, top) } else { (top, bottom) };

    let content = if sideways {
        vec2(first.x + gap + second.x, first.y.max(second.y))
    } else {
        vec2(first.x.max(second.x), first.y + gap + second.y)
    };
    let block = Rect::from_center_size(area.center(), content);

    // Each screen is centred across the shared axis, so screens of different
    // widths -- which different aspect ratios produce -- stay lined up.
    let (first_rect, second_rect) = if sideways {
        (
            Rect::from_min_size(pos2(block.left(), block.center().y - first.y / 2.0), first),
            Rect::from_min_size(
                pos2(block.left() + first.x + gap, block.center().y - second.y / 2.0),
                second,
            ),
        )
    } else {
        (
            Rect::from_min_size(pos2(block.center().x - first.x / 2.0, block.top()), first),
            Rect::from_min_size(
                pos2(block.center().x - second.x / 2.0, block.top() + first.y + gap),
                second,
            ),
        )
    };

    let (top_rect, bottom_rect) =
        if opts.swap { (second_rect, first_rect) } else { (first_rect, second_rect) };
    Layout { top: Some(top_rect), bottom: Some(bottom_rect) }
}
