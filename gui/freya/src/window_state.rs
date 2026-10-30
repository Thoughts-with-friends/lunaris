//! The window's place and size, kept per instance (`freya_window.json`) as
//! the egui GUI keeps its own, and never larger than the monitor it is on
//! (a window taller than the screen hides the status bar below it).
//!
//! ```text
//!  launch     restore(): the saved place on the monitor it was on (else the
//!             primary one), the size clamped to that monitor
//!  View > Screen size   fit_on_monitor(): the requested size, clamped
//!  quit / close         remember(): place, size, maximized
//! ```
use std::path::PathBuf;

use ds_common::Instance;
use freya::winit::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event_loop::ActiveEventLoop,
    monitor::MonitorHandle,
    window::{Window, WindowAttributes},
};
use serde::{Deserialize, Serialize};

/// Room kept for a taskbar / dock below the window (logical pixels).
const TASKBAR: f64 = 48.0;

/// Outer position (physical pixels), inner size (logical) and whether it
/// was maximized.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: f64,
    pub height: f64,
    pub maximized: bool,
}

fn path(instance: &Instance) -> PathBuf {
    instance.root().join("freya_window.json")
}

/// The state saved by [`remember`], if any.
pub fn load(instance: &Instance) -> Option<WindowState> {
    let text = std::fs::read_to_string(path(instance)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Saves where `window` is now. While maximized (or minimized) only the
/// flag changes: the size to come back to is the one before.
pub fn remember(instance: &Instance, window: &Window) {
    if window.is_minimized() == Some(true) {
        return;
    }
    let maximized = window.is_maximized();
    let saved = load(instance).filter(|_| maximized);
    let state = if let Some(saved) = saved {
        WindowState { maximized, ..saved }
    } else {
        let Ok(pos) = window.outer_position() else {
            return;
        };
        let size = window.inner_size().to_logical::<f64>(window.scale_factor());
        WindowState { x: pos.x, y: pos.y, width: size.width, height: size.height, maximized }
    };
    if let Ok(text) = serde_json::to_string_pretty(&state) {
        let _ = std::fs::write(path(instance), text);
    }
}

/// The largest inner size (logical) that fits on `monitor`.
fn room(monitor: &MonitorHandle) -> (f64, f64) {
    let size = monitor.size().to_logical::<f64>(monitor.scale_factor());
    (size.width, (size.height - TASKBAR).max(1.0))
}

/// `(w, h)` (logical), shrunk to fit the monitor `window` is on.
pub fn fit_on_monitor(window: &Window, w: f64, h: f64) -> (f64, f64) {
    window.current_monitor().map_or((w, h), |m| clamp((w, h), room(&m)))
}

const fn clamp((w, h): (f64, f64), (max_w, max_h): (f64, f64)) -> (f64, f64) {
    (w.min(max_w), h.min(max_h))
}

/// Whether physical point `(x, y)` lies on `monitor`.
fn contains(monitor: &MonitorHandle, x: i32, y: i32) -> bool {
    let (p, s) = (monitor.position(), monitor.size());
    let (w, h) =
        (i32::try_from(s.width).unwrap_or(i32::MAX), i32::try_from(s.height).unwrap_or(i32::MAX));
    (p.x..p.x.saturating_add(w)).contains(&x) && (p.y..p.y.saturating_add(h)).contains(&y)
}

/// The window's first attributes: the saved place and size (else
/// `default` logical), kept on a monitor.
pub fn restore(
    attrs: WindowAttributes,
    event_loop: &ActiveEventLoop,
    saved: Option<WindowState>,
    default: (f64, f64),
) -> WindowAttributes {
    let on = saved.and_then(|s| {
        // A little inside the window, so a title bar just off the edge counts.
        let (x, y) = (s.x.saturating_add(40), s.y.saturating_add(10));
        event_loop.available_monitors().find(|m| contains(m, x, y)).map(|m| (s, m))
    });
    let monitor = on.as_ref().map(|(_, m)| m.clone()).or_else(|| event_loop.primary_monitor());
    let wanted = saved.map_or(default, |s| (s.width, s.height));
    let (w, h) = monitor.as_ref().map_or(wanted, |m| clamp(wanted, room(m)));
    let mut attrs = attrs.with_inner_size(LogicalSize::new(w, h));
    if let Some((s, m)) = on {
        // Keep the whole window on that monitor.
        let scale = m.scale_factor();
        let size: PhysicalSize<i32> = LogicalSize::new(w, h + TASKBAR).to_physical(scale);
        let (p, ms) = (m.position(), m.size());
        let max_x = p.x + i32::try_from(ms.width).unwrap_or(i32::MAX) - size.width;
        let max_y = p.y + i32::try_from(ms.height).unwrap_or(i32::MAX) - size.height;
        let x = s.x.clamp(p.x, max_x.max(p.x));
        let y = s.y.clamp(p.y, max_y.max(p.y));
        attrs = attrs.with_position(PhysicalPosition::new(x, y)).with_maximized(s.maximized);
    }
    attrs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_clamped_to_the_room() {
        assert_eq!(clamp((800.0, 900.0), (1366.0, 720.0)), (800.0, 720.0));
        assert_eq!(clamp((560.0, 560.0), (1920.0, 1032.0)), (560.0, 560.0));
    }
}
