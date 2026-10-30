//! Pan/zoom gestures shared by the console view and the block design.
//!
//! ```text
//!  mouse wheel (line units)           zoom about the pointer
//!  touchpad two-finger scroll (points) pan
//!  pinch / Ctrl+wheel (zoom_delta)    zoom about the pointer
//!  primary drag on the background     pan (handled by each view)
//! ```

use eframe::egui::{self, MouseWheelUnit, Vec2};

pub struct Gesture {
    pub pan: Vec2,
    /// Multiplicative zoom, 1.0 = none.
    pub zoom: f32,
}

/// This repaint's wheel/touchpad/pinch input.
pub fn read(ui: &egui::Ui) -> Gesture {
    ui.input(|i| {
        let mut gesture = Gesture { pan: Vec2::ZERO, zoom: i.zoom_delta() };
        for event in &i.raw.events {
            if let egui::Event::MouseWheel { unit, delta, modifiers } = event {
                // Ctrl+wheel already arrives as `zoom_delta`.
                if modifiers.ctrl || modifiers.command {
                    continue;
                }
                match unit {
                    MouseWheelUnit::Point => gesture.pan += *delta,
                    MouseWheelUnit::Line | MouseWheelUnit::Page => {
                        gesture.zoom *= (delta.y * 0.12).exp();
                    }
                }
            }
        }
        gesture
    })
}

/// The pan that keeps the point under `anchor` fixed when the zoom goes from
/// `old` to `new` (`anchor` relative to the view origin).
pub fn pan_for_zoom(pan: Vec2, anchor: Vec2, old: f32, new: f32) -> Vec2 {
    anchor - (anchor - pan) * (new / old)
}
