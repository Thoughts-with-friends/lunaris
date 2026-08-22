//! What the View menu offers, and what a window remembers of it.

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
    pub(crate) const fn is_single(self) -> bool {
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
/// Persisted between runs (see `crate::file::settings`), so `serde` defaults every field:
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

/// The gaps melonDS offers, in DS pixels.
pub const SCREEN_GAPS: [u32; 6] = [0, 1, 8, 64, 90, 128];
