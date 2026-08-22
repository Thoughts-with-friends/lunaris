//! Emulation speed: the multiplier applied to the DS's own frame rate.
//!
//! The console itself is never reclocked. What changes is how many emulated
//! frames one repaint is allowed to run: at 2x the pacing loop earns frames
//! twice as fast, so the cart sees a wall clock that runs at half speed. That
//! is how melonDS's own fast-forward works, and it is the only approach that
//! keeps a frame bit-identical to the same frame at 1x.
//!
//! # Where it is *not* applied
//!
//! A speed other than 1x is refused while a second console or a LAN link is
//! live — see [`crate::app::MelonEgui::effective_speed`]. Both are two consoles
//! that must agree about time, and running one of them faster desynchronises
//! the game outright.

/// The multipliers the speed control steps through, in order.
///
/// The request's range, 0.5x to 4x, with the halves people actually use in
/// between. The pad's left-stick click walks this list, so it is deliberately
/// short: a long list is a lot of clicks to get back to 1x.
pub const STEPS: [f32; 6] = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0];

/// The slowest speed offered.
pub const MIN: f32 = 0.5;

/// The fastest speed offered. Past this the framerate limiter is the better
/// control: it runs the core as fast as the machine will go.
pub const MAX: f32 = 4.0;

/// Real time, and what a fresh settings file holds.
pub const DEFAULT: f32 = 1.0;

/// How close two speeds have to be to count as the same one.
///
/// The UI's slider produces arbitrary floats, so `==` is not usable for the
/// "is this real time?" tests that gate audio sync and the link-speed lock.
const EPSILON: f32 = 0.001;

/// Whether `speed` is real time.
#[must_use]
pub fn is_real_time(speed: f32) -> bool {
    (speed - DEFAULT).abs() < EPSILON
}

/// `speed` brought inside [`MIN`]..=[`MAX`], with a non-finite value — which a
/// hand-edited settings file can hold — read as [`DEFAULT`].
#[must_use]
pub fn clamp(speed: f32) -> f32 {
    if speed.is_finite() { speed.clamp(MIN, MAX) } else { DEFAULT }
}

/// The next entry of [`STEPS`] after `speed`, wrapping round to the first.
///
/// The step *after* whatever `speed` is nearest, so that cycling from a value
/// the slider produced — 1.75x, say — lands somewhere predictable rather than
/// on the first step every time.
#[must_use]
pub fn next(speed: f32) -> f32 {
    let nearest = STEPS
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (*a - speed).abs().total_cmp(&(*b - speed).abs()))
        .map_or(0, |(i, _)| i);
    STEPS[(nearest + 1) % STEPS.len()]
}

/// How a speed is written wherever one is shown.
#[must_use]
pub fn label(speed: f32) -> String {
    format!("{speed:.2}x")
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT, MAX, MIN, clamp, is_real_time, label, next};

    #[test]
    fn cycling_walks_the_steps_and_wraps() {
        assert_eq!(next(0.5), 1.0);
        assert_eq!(next(1.0), 1.5);
        assert_eq!(next(4.0), 0.5, "the last step wraps to the first");
    }

    #[test]
    fn cycling_from_a_slider_value_lands_on_the_step_after_the_nearest() {
        // 1.6 is nearest 1.5, so the click after it gives 2x.
        assert_eq!(next(1.6), 2.0);
    }

    #[test]
    fn a_hand_edited_file_cannot_put_the_speed_out_of_range() {
        assert_eq!(clamp(0.01), MIN);
        assert_eq!(clamp(99.0), MAX);
        assert_eq!(clamp(f32::NAN), DEFAULT);
        assert_eq!(clamp(2.0), 2.0);
    }

    #[test]
    fn real_time_tolerates_the_slider() {
        assert!(is_real_time(1.0));
        assert!(!is_real_time(1.5));
        assert_eq!(label(2.0), "2.00x");
    }
}
