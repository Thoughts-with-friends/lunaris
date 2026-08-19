//! Graphics settings: the model behind melonDS's **Config ▸ Video settings**
//! dialog.
//!
//! # What is reachable
//!
//! melonDS offers a choice of 3D renderer (software, OpenGL, compute) plus the
//! options that hang off each — internal resolution, better polygon splitting,
//! high-resolution coordinates, and threading for the software one. All of them
//! are reachable: `melonds-sys` builds the core with `ENABLE_OGLRENDERER` and
//! its FFI carries melonDS's `RendererSettings` whole, as
//! [`melonds::RenderSettings`].
//!
//! What the core cannot answer for is decided here instead:
//!
//! * Two core knobs about *compositing* rather than rasterising —
//!   `mds_set_render` and `mds_set_displayed_screens`, modelled as
//!   [`VideoOptions::render`] and [`VideoOptions::skip_hidden_screens`]. Both
//!   only skip work: melonDS documents emulation as bit-identical either way,
//!   including display capture into VRAM, so neither changes what a cart
//!   computes.
//! * Everything this front end does itself when it blits — filtering, aspect
//!   ratio, and vsync — which live in [`crate::view::ViewOptions`] and here.
//!
//! # Which of them a given machine can actually use
//!
//! An OpenGL renderer needs entry points bound against the window's context
//! (`melonds::gl_load`) and a driver new enough to compile melonDS's shaders;
//! the compute one additionally needs GL 4.3. None of that is knowable from a
//! settings file, so the choice is offered whenever the blitter came up and the
//! core reports back which renderer it actually installed — see
//! `MelonEgui::apply_render_settings`.

/// The 3D renderers melonDS's dialog offers.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum Renderer {
    #[default]
    Software,
    OpenGl,
    Compute,
}

impl Renderer {
    pub const ALL: [Self; 3] = [Self::Software, Self::OpenGl, Self::Compute];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Software => "Software",
            Self::OpenGl => "OpenGL",
            Self::Compute => "OpenGL (compute shader)",
        }
    }

    /// Whether this one draws into a GL texture rather than into host memory,
    /// which is what decides how the front end paints it and whether the
    /// OpenGL options apply.
    pub const fn is_gl(self) -> bool {
        matches!(self, Self::OpenGl | Self::Compute)
    }

    /// The core's name for it.
    pub const fn to_core(self) -> melonds::Renderer {
        match self {
            Self::Software => melonds::Renderer::Software,
            Self::OpenGl => melonds::Renderer::OpenGl,
            Self::Compute => melonds::Renderer::Compute,
        }
    }

    /// What the core said it installed, which is not always what was asked
    /// for: melonDS falls back to software rather than leave a console unable
    /// to draw.
    pub const fn from_core(renderer: melonds::Renderer) -> Self {
        match renderer {
            melonds::Renderer::Software => Self::Software,
            melonds::Renderer::OpenGl => Self::OpenGl,
            melonds::Renderer::Compute => Self::Compute,
        }
    }
}

/// Everything the Video settings dialog controls.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct VideoOptions {
    /// Which rasteriser the core draws with.
    pub renderer: Renderer,
    /// Internal resolution multiplier for the OpenGL renderers, 1 to 16.
    ///
    /// Unlike [`crate::view::ViewOptions::display_scale`], which magnifies the
    /// finished picture, this rasterises the 3D geometry itself at
    /// `256*n x 192*n` — so it adds real detail. Ignored by the software
    /// renderer, which has no such mode.
    pub internal_scale: u32,
    /// Run the software 3D rasteriser on worker threads. Faster on a machine
    /// with cores to spare, and off by default because melonDS ships it off.
    pub threaded_software: bool,
    /// "Improved polygon splitting" for the OpenGL renderer: fixes seams that
    /// upscaling opens in some geometry, at a cost in speed.
    pub better_polygons: bool,
    /// Hi-res vertex coordinates for the compute renderer, which keeps the
    /// extra precision upscaling makes visible instead of rounding to the DS's
    /// own grid.
    pub hires_coordinates: bool,
    /// Whether the core composites frames at all (`mds_set_render`).
    ///
    /// Off, emulation continues but the picture stops updating — which is what
    /// makes a fast-forward cheap. Deliberately not persisted as "off": a window
    /// that starts frozen looks broken.
    pub render: bool,
    /// Ask the core not to compose a screen the layout is hiding
    /// (`mds_set_displayed_screens`). Saves most of the 2D renderer's work in
    /// the single-screen sizings.
    pub skip_hidden_screens: bool,
    /// Post-process filter for the software renderer's finished picture.
    ///
    /// The only setting here that improves a *2D* layer: those are drawn from
    /// tiles at 256x192 whatever the renderer does, so
    /// [`Self::internal_scale`] cannot touch them. See [`crate::upscale`].
    pub upscale: crate::upscale::Method,
    /// How far [`Self::upscale`] scales, 1 to 6.
    pub upscale_factor: u8,
    /// Wait for the display's refresh before presenting. Applied when the window
    /// is created, so a change takes effect on the next run.
    pub vsync: bool,
}

impl Default for VideoOptions {
    fn default() -> Self {
        Self {
            renderer: Renderer::Software,
            internal_scale: 1,
            threaded_software: false,
            better_polygons: false,
            hires_coordinates: false,
            upscale: crate::upscale::Method::None,
            upscale_factor: 2,
            render: true,
            skip_hidden_screens: true,
            vsync: true,
        }
    }
}

impl VideoOptions {
    /// The upscaling factor the OpenGL renderers accept.
    pub const fn scale(self) -> u32 {
        // `clamp` is not const on integers in this edition; the range is the
        // core's own (`mds_set_render_settings` clamps to it as well).
        if self.internal_scale < 1 {
            1
        } else if self.internal_scale > 16 {
            16
        } else {
            self.internal_scale
        }
    }

    /// The xBRZ factor, held to what the filter accepts.
    pub const fn upscale_factor(self) -> u8 {
        crate::upscale::clamp_factor(self.upscale_factor)
    }

    /// These settings as the core takes them.
    ///
    /// Every field is carried whichever renderer is selected — melonDS ignores
    /// the ones its current renderer has no use for — so switching renderer
    /// and back restores the same picture rather than the defaults.
    pub const fn to_core(self) -> melonds::RenderSettings {
        melonds::RenderSettings {
            renderer: self.renderer.to_core(),
            scale: self.scale(),
            threaded: self.threaded_software,
            hires_coordinates: self.hires_coordinates,
            better_polygons: self.better_polygons,
        }
    }

    /// The `mds_set_displayed_screens` mask for a layout showing `top` and/or
    /// `bottom`: bit 0 is the top screen, bit 1 the bottom.
    ///
    /// Returns both screens when [`Self::skip_hidden_screens`] is off, so the
    /// setting is what decides whether the core is told anything at all.
    pub const fn displayed_mask(self, top: bool, bottom: bool) -> u8 {
        if !self.skip_hidden_screens {
            return 0b11;
        }
        let mut mask = 0;
        if top {
            mask |= 0b01;
        }
        if bottom {
            mask |= 0b10;
        }
        // Never tell the core that nothing is shown: with both engines idle the
        // framebuffers would freeze and the window would look hung.
        if mask == 0 { 0b11 } else { mask }
    }
}

#[cfg(test)]
mod tests {
    use super::{Renderer, VideoOptions};

    #[test]
    fn the_mask_follows_which_screens_are_shown() {
        let opts = VideoOptions::default();
        assert_eq!(opts.displayed_mask(true, true), 0b11);
        assert_eq!(opts.displayed_mask(true, false), 0b01, "top only");
        assert_eq!(opts.displayed_mask(false, true), 0b10, "bottom only");
    }

    #[test]
    fn hiding_both_screens_still_composes_something() {
        let opts = VideoOptions::default();
        assert_eq!(opts.displayed_mask(false, false), 0b11, "a frozen window looks broken");
    }

    #[test]
    fn the_setting_can_be_turned_off_entirely() {
        let opts = VideoOptions { skip_hidden_screens: false, ..Default::default() };
        assert_eq!(opts.displayed_mask(true, false), 0b11);
        assert_eq!(opts.displayed_mask(false, true), 0b11);
    }

    #[test]
    fn rendering_is_on_by_default_however_the_settings_file_was_written() {
        let opts: VideoOptions = serde_json::from_str("{}").unwrap();
        assert!(opts.render);
        assert!(opts.vsync);
    }

    #[test]
    fn every_renderer_setting_reaches_the_core() {
        let opts = VideoOptions {
            renderer: Renderer::Compute,
            internal_scale: 4,
            threaded_software: true,
            better_polygons: true,
            hires_coordinates: true,
            ..Default::default()
        };
        let core = opts.to_core();
        assert_eq!(core.renderer, melonds::Renderer::Compute);
        assert_eq!(core.scale, 4);
        assert!(core.threaded && core.better_polygons && core.hires_coordinates);
    }

    #[test]
    fn the_internal_resolution_is_held_to_what_the_core_accepts() {
        let too_big = VideoOptions { internal_scale: 99, ..Default::default() };
        assert_eq!(too_big.scale(), 16);
        let zero = VideoOptions { internal_scale: 0, ..Default::default() };
        assert_eq!(zero.scale(), 1);
    }

    #[test]
    fn a_settings_file_from_before_the_renderer_options_still_loads() {
        let opts: VideoOptions = serde_json::from_str(r#"{"renderer":"OpenGl"}"#).unwrap();
        assert_eq!(opts.renderer, Renderer::OpenGl);
        assert!(!opts.better_polygons, "absent fields take their defaults");
    }
}
