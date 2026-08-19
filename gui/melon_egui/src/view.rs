//! Screen presentation: the model behind melonDS's **View** menu, and the
//! layout arithmetic it drives.
//!
//! The names and the offered values are taken from melonDS's own menu
//! (`frontend/qt_sdl/Window.cpp`, the `View` block) so that the two front ends
//! can be set up the same way and their pictures compared directly. Every option
//! melonDS's View menu offers is implemented, with one documented simplification
//! in [`hybrid`].

use egui::{Rect, Vec2, pos2, vec2};
use melonds::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// Quarter turns clockwise applied to both screens.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum Rotation {
    #[default]
    None,
    Cw90,
    Cw180,
    Cw270,
}

impl Rotation {
    pub const ALL: [Self; 4] = [Self::None, Self::Cw90, Self::Cw180, Self::Cw270];

    /// The angle melonDS labels this entry with.
    pub const fn degrees(self) -> u32 {
        self.steps() as u32 * 90
    }

    /// Quarter turns, which is also how far the texture coordinates rotate.
    pub const fn steps(self) -> usize {
        match self {
            Self::None => 0,
            Self::Cw90 => 1,
            Self::Cw180 => 2,
            Self::Cw270 => 3,
        }
    }

    /// Whether a screen ends up wider than it is tall, which is what decides
    /// the `Natural` layout and swaps each screen's width and height.
    pub const fn is_sideways(self) -> bool {
        matches!(self, Self::Cw90 | Self::Cw270)
    }
}

/// How the two screens are arranged relative to each other.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum ScreenLayout {
    /// Stacked, or side by side once rotated — whichever way up the console is.
    #[default]
    Natural,
    Vertical,
    Horizontal,
    /// One screen large with the other beside it, small.
    Hybrid,
}

impl ScreenLayout {
    pub const ALL: [Self; 4] = [Self::Natural, Self::Vertical, Self::Horizontal, Self::Hybrid];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Natural => "Natural",
            Self::Vertical => "Vertical",
            Self::Horizontal => "Horizontal",
            Self::Hybrid => "Hybrid",
        }
    }
}

/// How the available room is divided between the screens.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum ScreenSizing {
    /// Both screens at the same scale.
    #[default]
    Even,
    /// The top screen as large as it will go, the bottom fitted into what is
    /// left.
    EmphasizeTop,
    /// The same, the other way round.
    EmphasizeBottom,
    /// Show only whichever screen the console is actually drawing to, falling
    /// back to `Even` while both are live. Resolved by
    /// [`ScreenSizing::resolve`] before it reaches the layout.
    Auto,
    TopOnly,
    BottomOnly,
}

impl ScreenSizing {
    pub const ALL: [Self; 6] = [
        Self::Even,
        Self::EmphasizeTop,
        Self::EmphasizeBottom,
        Self::Auto,
        Self::TopOnly,
        Self::BottomOnly,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Even => "Even",
            Self::EmphasizeTop => "Emphasize top",
            Self::EmphasizeBottom => "Emphasize bottom",
            Self::Auto => "Auto",
            Self::TopOnly => "Top only",
            Self::BottomOnly => "Bottom only",
        }
    }

    /// Whether only one screen is drawn.
    const fn is_single(self) -> bool {
        matches!(self, Self::TopOnly | Self::BottomOnly)
    }

    /// Turn `Auto` into a concrete sizing, given which screens the console is
    /// drawing anything to.
    ///
    /// melonDS resolves `Auto` outside its layout code too (`ScreenLayout.h`:
    /// "not applied in SetupScreenLayout"), because it depends on emulator state
    /// rather than on the window. A screen with nothing on it is one worth
    /// giving up to the other.
    pub const fn resolve(self, top_live: bool, bottom_live: bool) -> Self {
        match self {
            Self::Auto => match (top_live, bottom_live) {
                (true, false) => Self::TopOnly,
                (false, true) => Self::BottomOnly,
                _ => Self::Even,
            },
            other => other,
        }
    }
}

/// The gaps melonDS offers, in DS pixels.
pub const SCREEN_GAPS: [u32; 6] = [0, 1, 8, 64, 90, 128];

/// How wide a screen is drawn relative to the DS's native 4:3.
///
/// Values transcribed from melonDS's `aspectRatios[]` (`frontend/qt_sdl/
/// Screen.h`), which stores each as a multiple of 4:3 and applies it per screen.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum AspectRatio {
    #[default]
    Native,
    /// 5:3, the 3DS's top screen.
    Wide5x3,
    Wide16x9,
    Wide21x9,
    /// Stretch to whatever the window itself is.
    Window,
}

impl AspectRatio {
    pub const ALL: [Self; 5] =
        [Self::Native, Self::Wide5x3, Self::Wide16x9, Self::Wide21x9, Self::Window];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Native => "4:3 (native)",
            Self::Wide5x3 => "5:3 (3DS)",
            Self::Wide16x9 => "16:9",
            Self::Wide21x9 => "21:9",
            Self::Window => "window",
        }
    }

    /// How much wider than native, or `None` for [`Self::Window`], whose factor
    /// depends on the window and so is only known at layout time.
    pub fn multiplier(self) -> Option<f32> {
        Some(match self {
            Self::Native => 1.0,
            Self::Wide5x3 => (5.0 / 3.0) / (4.0 / 3.0),
            Self::Wide16x9 => (16.0 / 9.0) / (4.0 / 3.0),
            Self::Wide21x9 => (21.0 / 9.0) / (4.0 / 3.0),
            Self::Window => return None,
        })
    }
}

/// Everything the View menu controls.
///
/// Persisted between runs (see `crate::config`), so `serde` defaults every field:
/// a settings file written by an older build must still load.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ViewOptions {
    pub rotation: Rotation,
    /// Space between the screens, in DS pixels, scaled with them.
    pub gap: u32,
    pub layout: ScreenLayout,
    /// Draw the bottom screen where the top one would go, and vice versa.
    pub swap: bool,
    pub sizing: ScreenSizing,
    /// How wide each screen is drawn. melonDS sets these per screen, so a
    /// widescreen hack can stretch the 3D screen and leave the other native.
    pub aspect_top: AspectRatio,
    pub aspect_bottom: AspectRatio,
    /// Draw the screens at exactly this magnification instead of fitting them to
    /// the window. `None` fits, which is the default.
    ///
    /// This is *display* scaling — the GPU samples the 256x192 framebuffer at
    /// this factor. It is not melonDS's "internal resolution", which re-renders
    /// 3D geometry at a higher resolution inside the OpenGL renderer; see
    /// [`crate::video`] for why that one is out of reach here.
    pub display_scale: Option<f32>,
    /// Restrict the fitted scale to whole numbers, so every DS pixel covers the
    /// same number of screen pixels.
    pub integer_scaling: bool,
    /// Smooth the picture when scaled, rather than showing square pixels.
    pub filtering: bool,
    pub show_osd: bool,
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            rotation: Rotation::default(),
            // melonDS's own default gap is none.
            gap: 0,
            layout: ScreenLayout::default(),
            swap: false,
            sizing: ScreenSizing::default(),
            display_scale: None,
            aspect_top: AspectRatio::default(),
            aspect_bottom: AspectRatio::default(),
            // Square pixels by default: this front end exists to be compared
            // against, and interpolation would blur what is being compared.
            integer_scaling: false,
            filtering: false,
            show_osd: true,
        }
    }
}

/// Where each screen is drawn. `None` means that screen is not shown, which
/// only [`ScreenSizing::TopOnly`] and [`ScreenSizing::BottomOnly`] cause.
#[derive(Debug, PartialEq)]
pub struct Layout {
    pub top: Option<Rect>,
    pub bottom: Option<Rect>,
}

/// The aspect-ratio multiplier for one screen, resolving
/// [`AspectRatio::Window`] against the area being filled.
fn aspect_multiplier(aspect: AspectRatio, area: Rect) -> f32 {
    aspect.multiplier().unwrap_or_else(|| {
        // "window": as wide relative to native as the window itself is.
        let window = area.width() / area.height().max(1.0);
        (window / (4.0 / 3.0)).max(0.1)
    })
}

/// One screen's size in DS pixels, with its aspect ratio applied and the
/// rotation taken into account: a quarter turn puts it on its side.
fn screen_size(opts: &ViewOptions, aspect: AspectRatio, area: Rect) -> Vec2 {
    let width = SCREEN_WIDTH as f32 * aspect_multiplier(aspect, area);
    if opts.rotation.is_sideways() {
        vec2(SCREEN_HEIGHT as f32, width)
    } else {
        vec2(width, SCREEN_HEIGHT as f32)
    }
}

/// The two screens' sizes as `(top, bottom)`.
fn screen_sizes(opts: &ViewOptions, area: Rect) -> (Vec2, Vec2) {
    (screen_size(opts, opts.aspect_top, area), screen_size(opts, opts.aspect_bottom, area))
}

/// Whether the two screens sit beside each other rather than stacked.
///
/// `Natural` means "as the console is held": rotated onto its side, the screens
/// end up next to each other.
fn is_side_by_side(opts: &ViewOptions) -> bool {
    match opts.layout {
        ScreenLayout::Vertical => false,
        ScreenLayout::Horizontal => true,
        // Hybrid always puts its small screen beside the large one.
        ScreenLayout::Hybrid => true,
        ScreenLayout::Natural => opts.rotation.is_sideways(),
    }
}

/// The bounding box of everything drawn, in DS pixels at 1x.
fn content_size(opts: &ViewOptions, area: Rect) -> Vec2 {
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

/// The largest scale at which `size` fits inside `area`, honouring
/// `integer_scaling`.
fn fit(area: Rect, size: Vec2, opts: &ViewOptions) -> f32 {
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

/// Both screens at one shared scale, the pair centred in `area`.
fn even(area: Rect, opts: &ViewOptions, top_size: Vec2, bottom_size: Vec2) -> Layout {
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
fn emphasized(area: Rect, opts: &ViewOptions, top_size: Vec2, bottom_size: Vec2) -> Layout {
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
fn hybrid(area: Rect, opts: &ViewOptions, top_size: Vec2, bottom_size: Vec2) -> Layout {
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
fn place_pair(area: Rect, opts: &ViewOptions, top: Vec2, bottom: Vec2, gap: f32) -> Layout {
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

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::*;

    const W: f32 = SCREEN_WIDTH as f32;
    const H: f32 = SCREEN_HEIGHT as f32;

    /// An area exactly 2x the stacked screens, so the expected numbers are
    /// obvious rather than derived.
    fn area_2x() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(W * 2.0, H * 4.0))
    }

    #[test]
    fn even_vertical_stacks_the_screens_with_no_gap() {
        let opts = ViewOptions { layout: ScreenLayout::Vertical, ..Default::default() };
        let placed = layout(area_2x(), &opts);
        assert_eq!(placed.top, Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(W * 2.0, H * 2.0))));
        assert_eq!(
            placed.bottom,
            Some(Rect::from_min_size(pos2(0.0, H * 2.0), vec2(W * 2.0, H * 2.0)))
        );
    }

    #[test]
    fn a_gap_costs_scale_and_appears_between_the_screens() {
        let opts = ViewOptions { layout: ScreenLayout::Vertical, gap: 64, ..Default::default() };
        let placed = layout(area_2x(), &opts).top.zip(layout(area_2x(), &opts).bottom);
        let (top, bottom) = placed.expect("both screens shown");
        // The gap has to fit too, so the screens are smaller than 2x.
        assert!(top.height() < H * 2.0, "{top:?}");
        // And it lands between them, scaled the same as the screens.
        let scale = top.height() / H;
        assert!((bottom.top() - top.bottom() - 64.0 * scale).abs() < 0.01);
    }

    #[test]
    fn horizontal_puts_them_side_by_side() {
        let opts = ViewOptions { layout: ScreenLayout::Horizontal, ..Default::default() };
        let placed = layout(area_2x(), &opts);
        let (top, bottom) = (placed.top.unwrap(), placed.bottom.unwrap());
        assert_eq!(top.top(), bottom.top(), "same row");
        assert_eq!(top.right(), bottom.left(), "no gap requested");
    }

    #[test]
    fn natural_follows_the_rotation() {
        let upright =
            layout(area_2x(), &ViewOptions { layout: ScreenLayout::Natural, ..Default::default() });
        assert_eq!(
            upright.top.unwrap().top(),
            0.0,
            "upright, the top screen is above the bottom one",
        );
        assert!(upright.top.unwrap().bottom() <= upright.bottom.unwrap().top());

        let sideways = layout(
            area_2x(),
            &ViewOptions {
                layout: ScreenLayout::Natural,
                rotation: Rotation::Cw90,
                ..Default::default()
            },
        );
        let (top, bottom) = (sideways.top.unwrap(), sideways.bottom.unwrap());
        assert_eq!(top.top(), bottom.top(), "on its side, they share a row");
        // A rotated screen is taller than it is wide.
        assert!(top.height() > top.width(), "{top:?}");
    }

    #[test]
    fn swap_exchanges_the_two_places() {
        let opts = ViewOptions { layout: ScreenLayout::Vertical, ..Default::default() };
        let plain = layout(area_2x(), &opts);
        let swapped = layout(area_2x(), &ViewOptions { swap: true, ..opts });
        assert_eq!(plain.top, swapped.bottom);
        assert_eq!(plain.bottom, swapped.top);
    }

    #[test]
    fn top_only_hides_the_other_screen_and_swap_picks_the_other_one() {
        let opts = ViewOptions { sizing: ScreenSizing::TopOnly, ..Default::default() };
        let placed = layout(area_2x(), &opts);
        assert!(placed.top.is_some());
        assert_eq!(placed.bottom, None);

        let swapped = layout(area_2x(), &ViewOptions { swap: true, ..opts });
        assert_eq!(swapped.top, None);
        assert!(swapped.bottom.is_some());

        let bottom_only = layout(
            area_2x(),
            &ViewOptions { sizing: ScreenSizing::BottomOnly, ..Default::default() },
        );
        assert_eq!(bottom_only.top, None);
        assert!(bottom_only.bottom.is_some());
    }

    #[test]
    fn integer_scaling_floors_the_fit() {
        // Room for 2.5x, which without the restriction is what it would use.
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(W * 2.5, H * 5.0));
        let loose =
            layout(area, &ViewOptions { layout: ScreenLayout::Vertical, ..Default::default() });
        assert_eq!(loose.top.unwrap().width(), W * 2.5);

        let strict = layout(
            area,
            &ViewOptions {
                layout: ScreenLayout::Vertical,
                integer_scaling: true,
                ..Default::default()
            },
        );
        assert_eq!(strict.top.unwrap().width(), W * 2.0);
    }

    #[test]
    fn an_explicit_display_scale_overrides_the_fit() {
        // An area big enough for 4x, asked to draw at 2x.
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(W * 4.0, H * 8.0));
        let placed = layout(
            area,
            &ViewOptions {
                layout: ScreenLayout::Vertical,
                display_scale: Some(2.0),
                ..Default::default()
            },
        );
        let top = placed.top.unwrap();
        assert_eq!(top.width(), W * 2.0);
        assert_eq!(top.height(), H * 2.0);
        // Still centred, with the surplus as a border.
        let block = top.union(placed.bottom.unwrap());
        assert!((block.center().y - area.center().y).abs() < 0.01);
    }

    #[test]
    fn an_explicit_scale_is_not_rounded_by_integer_scaling() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(W * 8.0, H * 16.0));
        let placed = layout(
            area,
            &ViewOptions {
                layout: ScreenLayout::Vertical,
                display_scale: Some(2.5),
                integer_scaling: true,
                ..Default::default()
            },
        );
        assert_eq!(placed.top.unwrap().width(), W * 2.5, "an explicit scale is taken as given");
    }

    #[test]
    fn an_explicit_scale_larger_than_the_window_is_allowed() {
        // Overflow is a crop, not a squeeze: the panel clips it.
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(W, H));
        let placed = layout(
            area,
            &ViewOptions {
                layout: ScreenLayout::Vertical,
                display_scale: Some(4.0),
                ..Default::default()
            },
        );
        assert_eq!(placed.top.unwrap().width(), W * 4.0);
    }

    #[test]
    fn the_screens_stay_centred_in_a_larger_area() {
        let area = Rect::from_min_size(pos2(10.0, 20.0), vec2(W * 8.0, H * 4.0));
        let placed =
            layout(area, &ViewOptions { layout: ScreenLayout::Vertical, ..Default::default() });
        let block = placed.top.unwrap().union(placed.bottom.unwrap());
        assert!((block.center().x - area.center().x).abs() < 0.01);
        assert!((block.center().y - area.center().y).abs() < 0.01);
    }

    #[test]
    fn a_degenerate_area_does_not_panic() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), Vec2::ZERO);
        let placed = layout(area, &ViewOptions::default());
        assert_eq!(placed.top.unwrap().area(), 0.0);
    }

    #[test]
    fn emphasize_top_makes_the_top_screen_the_larger_one() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(W * 3.0, H * 5.0));
        let placed = layout(
            area,
            &ViewOptions {
                layout: ScreenLayout::Vertical,
                sizing: ScreenSizing::EmphasizeTop,
                ..Default::default()
            },
        );
        let (top, bottom) = (placed.top.unwrap(), placed.bottom.unwrap());
        assert!(top.height() > bottom.height(), "top {top:?} bottom {bottom:?}");
        // The secondary keeps at least its native size, per melonDS's rule.
        assert!(bottom.height() >= H - 0.01, "{bottom:?}");
        // And the pair still fits.
        assert!(top.height() + bottom.height() <= area.height() + 0.01);
    }

    #[test]
    fn emphasize_bottom_is_the_mirror_of_emphasize_top() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(W * 3.0, H * 5.0));
        let make = |sizing| {
            layout(
                area,
                &ViewOptions { layout: ScreenLayout::Vertical, sizing, ..Default::default() },
            )
        };
        let top_emph = make(ScreenSizing::EmphasizeTop);
        let bottom_emph = make(ScreenSizing::EmphasizeBottom);
        assert_eq!(top_emph.top.unwrap().size(), bottom_emph.bottom.unwrap().size());
        assert_eq!(top_emph.bottom.unwrap().size(), bottom_emph.top.unwrap().size());
    }

    #[test]
    fn auto_gives_the_window_to_whichever_screen_is_live() {
        let auto = ScreenSizing::Auto;
        assert_eq!(auto.resolve(true, false), ScreenSizing::TopOnly);
        assert_eq!(auto.resolve(false, true), ScreenSizing::BottomOnly);
        assert_eq!(auto.resolve(true, true), ScreenSizing::Even, "both live: share");
        assert_eq!(auto.resolve(false, false), ScreenSizing::Even, "neither: no reason to pick");
        // Any concrete sizing passes straight through.
        assert_eq!(ScreenSizing::TopOnly.resolve(false, true), ScreenSizing::TopOnly);
    }

    #[test]
    fn hybrid_makes_one_screen_larger_and_puts_them_side_by_side() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(W * 6.0, H * 3.0));
        let placed =
            layout(area, &ViewOptions { layout: ScreenLayout::Hybrid, ..Default::default() });
        let (top, bottom) = (placed.top.unwrap(), placed.bottom.unwrap());
        assert!(top.width() > bottom.width(), "the top screen is the large one");
        assert!(top.right() <= bottom.left() + 0.01, "large on the left");
        // 4:3 is the ratio melonDS uses between the panels.
        assert!((top.width() / bottom.width() - 4.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn hybrid_swap_promotes_the_bottom_screen() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(W * 6.0, H * 3.0));
        let placed = layout(
            area,
            &ViewOptions { layout: ScreenLayout::Hybrid, swap: true, ..Default::default() },
        );
        let (top, bottom) = (placed.top.unwrap(), placed.bottom.unwrap());
        assert!(bottom.width() > top.width(), "swapped, the bottom screen is the large one");
    }

    #[test]
    fn a_wider_aspect_ratio_widens_only_the_screen_it_is_set_on() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(W * 4.0, H * 4.0));
        let native =
            layout(area, &ViewOptions { layout: ScreenLayout::Vertical, ..Default::default() });
        let wide = layout(
            area,
            &ViewOptions {
                layout: ScreenLayout::Vertical,
                aspect_top: AspectRatio::Wide16x9,
                ..Default::default()
            },
        );
        let (n_top, w_top) = (native.top.unwrap(), wide.top.unwrap());
        let w_bottom = wide.bottom.unwrap();
        // The top screen is now wider relative to its own height...
        assert!(w_top.aspect_ratio() > n_top.aspect_ratio(), "{w_top:?} vs {n_top:?}");
        // ...and wider than the bottom one, which was left native.
        assert!(w_top.width() > w_bottom.width());
        // Both stay centred on the same vertical line.
        assert!((w_top.center().x - w_bottom.center().x).abs() < 0.01);
    }

    #[test]
    fn the_window_aspect_ratio_follows_the_area_it_is_given() {
        // A wide area should stretch a `Window`-aspect screen wider than native.
        let wide_area = Rect::from_min_size(pos2(0.0, 0.0), vec2(2000.0, 500.0));
        let placed = layout(
            wide_area,
            &ViewOptions {
                layout: ScreenLayout::Vertical,
                aspect_top: AspectRatio::Window,
                aspect_bottom: AspectRatio::Window,
                ..Default::default()
            },
        );
        let top = placed.top.unwrap();
        assert!(top.aspect_ratio() > (W / H) * 1.5, "{top:?}");
    }

    #[test]
    fn window_size_for_scale_matches_what_the_layout_then_fits() {
        for opts in [
            ViewOptions::default(),
            ViewOptions { layout: ScreenLayout::Horizontal, ..Default::default() },
            ViewOptions { rotation: Rotation::Cw90, ..Default::default() },
        ] {
            let size = window_size_for_scale(3.0, &opts, 0.0);
            let placed = layout(Rect::from_min_size(pos2(0.0, 0.0), size), &opts);
            let drawn = placed.top.unwrap();
            let expected = if opts.rotation.is_sideways() { H } else { W };
            assert!(
                (drawn.width() - expected * 3.0).abs() < 0.5,
                "{opts:?} gave {drawn:?}",
                opts = opts.layout,
            );
        }
    }
}
