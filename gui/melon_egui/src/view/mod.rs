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

mod measure;
mod options;
mod place;

pub use measure::window_size_for_scale;
pub(crate) use measure::{content_size, fit, is_side_by_side, screen_sizes};
pub use options::{AspectRatio, Rotation, SCREEN_GAPS, ScreenLayout, ScreenSizing, ViewOptions};
pub use place::layout;

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
