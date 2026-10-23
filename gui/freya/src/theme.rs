//! Colors and the freya theme: a dark, translucent "glass" look over the
//! transparent window, with one accent color.
use freya::prelude::*;

/// Accent (buttons, selection, checked menu entries).
pub const ACCENT: (u8, u8, u8) = (124, 140, 255);
/// Accent for gradients' second stop.
pub const ACCENT_2: (u8, u8, u8) = (198, 120, 255);

pub const TEXT: (u8, u8, u8) = (232, 234, 245);
pub const TEXT_DIM: (u8, u8, u8) = (150, 156, 180);
pub const TEXT_FAINT: (u8, u8, u8) = (105, 110, 135);
pub const SUCCESS: (u8, u8, u8) = (96, 214, 150);
pub const WARNING: (u8, u8, u8) = (255, 196, 90);
pub const DANGER: (u8, u8, u8) = (255, 105, 120);

/// Window body tint; its alpha is the user's opacity setting.
pub const BODY: (u8, u8, u8) = (14, 16, 26);

/// A panel slightly lighter than the body.
pub const fn panel() -> Color {
    Color::from_argb(150, 30, 33, 50)
}

/// A raised surface (cards, inputs).
pub const fn surface() -> Color {
    Color::from_argb(200, 38, 42, 62)
}

/// Hover state of [`surface`].
pub const fn surface_hover() -> Color {
    Color::from_argb(220, 52, 57, 84)
}

/// Thin separators and outlines.
pub const fn hairline() -> Color {
    Color::from_argb(40, 255, 255, 255)
}

pub const fn rgb(c: (u8, u8, u8)) -> Color {
    Color::from_rgb(c.0, c.1, c.2)
}

pub const fn rgba(c: (u8, u8, u8), alpha: u8) -> Color {
    Color::from_argb(alpha, c.0, c.1, c.2)
}

/// freya's dark theme with the accent colors and translucent surfaces, so
/// the built-in components (menus, buttons, inputs, switches) match.
pub fn lunaris_theme() -> Theme {
    let mut theme = dark_theme();
    theme.name = "lunaris";
    theme.colors = ColorsSheet {
        primary: rgb(ACCENT),
        secondary: rgb(ACCENT_2),
        tertiary: Color::from_rgb(92, 104, 210),
        // Menus, tooltips and popups draw on this: nearly opaque.
        background: Color::from_argb(248, 24, 27, 40),
        surface_primary: surface(),
        surface_secondary: panel(),
        surface_tertiary: surface_hover(),
        border: hairline(),
        border_focus: rgb(ACCENT),
        text_primary: rgb(TEXT),
        text_secondary: rgb(TEXT_DIM),
        text_placeholder: rgb(TEXT_FAINT),
        ..DARK_COLORS
    };
    theme
}
