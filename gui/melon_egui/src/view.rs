//! Screen presentation: the model behind melonDS's **View** menu, and the
//! layout arithmetic it drives.
//!
//! The names and the offered values are taken from melonDS's own menu
//! (`frontend/qt_sdl/Window.cpp`, the `View` block) so that the two front ends
//! can be set up the same way and their pictures compared directly. Options
//! this front end does not implement are still listed by the menu, greyed out —
//! see [`ScreenLayout::supported`] and [`ScreenSizing::supported`].

use egui::{Rect, Vec2, vec2};
use melonds::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// Quarter turns clockwise applied to both screens.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ScreenLayout {
    /// Stacked, or side by side once rotated — whichever way up the console is.
    #[default]
    Natural,
    Vertical,
    Horizontal,
    /// One screen large with the other inset. Not implemented here.
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

    pub const fn supported(self) -> bool {
        !matches!(self, Self::Hybrid)
    }
}

/// How the available room is divided between the screens.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ScreenSizing {
    /// Both screens at the same scale.
    #[default]
    Even,
    /// Not implemented here.
    EmphasizeTop,
    /// Not implemented here.
    EmphasizeBottom,
    /// Not implemented here.
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

    pub const fn supported(self) -> bool {
        matches!(self, Self::Even | Self::TopOnly | Self::BottomOnly)
    }

    /// Whether only one screen is drawn.
    const fn is_single(self) -> bool {
        matches!(self, Self::TopOnly | Self::BottomOnly)
    }
}

/// The gaps melonDS offers, in DS pixels.
pub const SCREEN_GAPS: [u32; 6] = [0, 1, 8, 64, 90, 128];

/// Everything the View menu controls.
#[derive(Clone, Copy)]
pub struct ViewOptions {
    pub rotation: Rotation,
    /// Space between the screens, in DS pixels, scaled with them.
    pub gap: u32,
    pub layout: ScreenLayout,
    /// Draw the bottom screen where the top one would go, and vice versa.
    pub swap: bool,
    pub sizing: ScreenSizing,
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

/// One screen's size in DS pixels once rotated: a quarter turn puts it on its
/// side.
fn screen_size(opts: &ViewOptions) -> Vec2 {
    if opts.rotation.is_sideways() {
        vec2(SCREEN_HEIGHT as f32, SCREEN_WIDTH as f32)
    } else {
        vec2(SCREEN_WIDTH as f32, SCREEN_HEIGHT as f32)
    }
}

/// Whether the two screens sit beside each other rather than stacked.
///
/// `Natural` means "as the console is held": rotated onto its side, the screens
/// end up next to each other.
fn is_side_by_side(opts: &ViewOptions) -> bool {
    match opts.layout {
        ScreenLayout::Vertical => false,
        ScreenLayout::Horizontal => true,
        ScreenLayout::Natural | ScreenLayout::Hybrid => opts.rotation.is_sideways(),
    }
}

/// The bounding box of everything drawn, in DS pixels — both screens plus the
/// gap, or one screen when only one is shown.
fn content_size(opts: &ViewOptions) -> Vec2 {
    let screen = screen_size(opts);
    let gap = opts.gap as f32;
    if opts.sizing.is_single() {
        screen
    } else if is_side_by_side(opts) {
        vec2(screen.x * 2.0 + gap, screen.y)
    } else {
        vec2(screen.x, screen.y * 2.0 + gap)
    }
}

/// Fit the screens into `area`.
///
/// Both screens always get the same scale — the largest that fits — and the
/// result is centred, leaving any surplus as an even border. `area` is expected
/// to be the whole central panel; a degenerate one yields empty rectangles
/// rather than a panic.
pub fn layout(area: Rect, opts: &ViewOptions) -> Layout {
    let screen = screen_size(opts);
    let single = opts.sizing.is_single();
    let side_by_side = is_side_by_side(opts);
    let gap = opts.gap as f32;
    let content = content_size(opts);

    let mut scale = (area.width() / content.x).min(area.height() / content.y);
    if !scale.is_finite() || scale <= 0.0 {
        scale = 0.0;
    }
    if opts.integer_scaling {
        // Never below 1x: a window too small for one whole pixel per pixel is
        // better overflowing than blank.
        scale = scale.floor().max(1.0);
    }

    let block = Rect::from_center_size(area.center(), content * scale);
    let screen = screen * scale;

    if single {
        let shown = Some(block);
        // Swapping with one screen shown selects the other screen, matching
        // melonDS: the choice is which panel occupies the window.
        let top_shown = (opts.sizing == ScreenSizing::TopOnly) != opts.swap;
        return Layout {
            top: top_shown.then_some(shown).flatten(),
            bottom: (!top_shown).then_some(shown).flatten(),
        };
    }

    let step = if side_by_side {
        vec2(screen.x + gap * scale, 0.0)
    } else {
        vec2(0.0, screen.y + gap * scale)
    };
    let first = Rect::from_min_size(block.min, screen);
    let second = Rect::from_min_size(block.min + step, screen);

    let (top, bottom) = if opts.swap { (second, first) } else { (first, second) };
    Layout { top: Some(top), bottom: Some(bottom) }
}

/// The window size that shows both screens at exactly `scale`, plus `chrome`
/// for the menu bar and status line.
///
/// melonDS's "Screen size" entries resize the window rather than setting a
/// zoom, so the fitted scale then works out to exactly what was asked for.
pub fn window_size_for_scale(scale: f32, opts: &ViewOptions, chrome: f32) -> Vec2 {
    let content = content_size(opts) * scale;
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
