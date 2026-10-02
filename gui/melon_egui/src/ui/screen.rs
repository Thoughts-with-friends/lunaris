//! The first console's picture in the main window.
//!
//! # Two routes, depending on the renderer
//!
//! ```text
//!  software renderer:  framebuffers (CPU) ─ upload ─→ egui textures ─ screens ─→ window
//!                      (xBRZ, if on, runs inside to_image)
//!
//!  OpenGL renderer:    core's GL texture ─ screens (paint callback) ─→ window
//!                      2D layer ─ filter_gl_2d: read back 256x192, xBRZ on
//!                      the CPU, upload ─→ shader blends it over the 3D
//! ```

use crate::app::*;

impl MelonEgui {
    /// The pointer on the bottom screen in touchscreen coordinates, or `None`
    /// when the stylus is not down on it. Uses last repaint's screen rectangle.
    pub(crate) fn sample_touch(&self, ctx: &egui::Context) -> Option<(u16, u16)> {
        let rect = self.bottom_screen?;
        let pos =
            ctx.input(|i| i.pointer.primary_down().then(|| i.pointer.interact_pos()).flatten())?;
        touch_coords(rect, pos, self.view.rotation)
    }

    /// Move the newly drawn frame to the GPU (called after frames ran).
    pub(crate) fn upload(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(emu) = &mut self.emu else {
            return;
        };
        if emu.nds.gl_output().is_some() {
            self.filter_gl_2d(frame);
            // `ScreenSizing::Auto` cannot inspect a GPU texture cheaply, so
            // under OpenGL both screens count as live.
            self.screens_live = [true, true];
            return;
        }
        let Some((top, bottom)) = emu.nds.framebuffers() else {
            return;
        };
        // What `ScreenSizing::Auto` decides on: an all-black screen is unused.
        self.screens_live = [crate::emu::has_picture(top), crate::emu::has_picture(bottom)];
        upload_screens(
            ctx,
            &mut self.textures,
            ["ds-top", "ds-bottom"],
            [top, bottom],
            &self.video,
            &self.view,
        );
    }

    /// Run the real xBRZ over the OpenGL renderer's 2D content.
    ///
    /// The 2D layers live at 256x192 whatever the internal resolution, so only
    /// one texel per DS pixel is read back (196 KB a screen), filtered on the
    /// CPU exactly as the software route does, and handed back to the shader.
    /// The 3D never makes the trip and keeps every pixel the GPU drew. See
    /// [`crate::gl_screen::capture`].
    fn filter_gl_2d(&mut self, frame: &eframe::Frame) {
        let Some(screen) = self.gl_screen.clone() else { return };
        let Some(gl) = frame.gl() else { return };
        let Some(output) = self.emu.as_mut().and_then(|emu| emu.nds.gl_output()) else { return };

        if self.video.upscale == crate::upscale::Method::None || !screen.can_filter() {
            // So switching the filter off cannot leave one more filtered frame.
            screen.invalidate_filtered();
            return;
        }

        let (method, factor) = (self.video.upscale, self.video.upscale_factor());
        for layer in 0..2u32 {
            let Some(rgba) = screen.read_ds_pixels(gl, output.texture, layer) else { continue };
            let (filtered, width, height) = crate::upscale::upscale(
                rgba,
                crate::gl_screen::DS_WIDTH as usize,
                crate::gl_screen::DS_HEIGHT as usize,
                method,
                factor,
            );
            screen.write_filtered(gl, layer, &filtered, width as u32, height as u32);
        }
    }

    /// Lay the screens out in `area` and paint them, remembering where the
    /// bottom one landed for the next repaint's touch mapping.
    pub(crate) fn screens(&mut self, ui: &mut egui::Ui, area: Rect) {
        let placed = view::layout(area, &self.resolved_view());
        self.bottom_screen = placed.bottom;

        // OpenGL: the picture is a texture in eframe's own GL context, drawn
        // by a callback inside egui's painter.
        if let Some(output) = self.emu.as_mut().and_then(|emu| emu.nds.gl_output())
            && let Some(screen) = self.gl_screen.clone()
        {
            let filter =
                if self.view.filtering { eframe::glow::LINEAR } else { eframe::glow::NEAREST };
            let smooth = self.video.upscale == crate::upscale::Method::Xbrz;
            for (rect, layer) in [(placed.top, 0.0f32), (placed.bottom, 1.0f32)] {
                let Some(rect) = rect else { continue };
                let screen = screen.clone();
                let callback = egui_glow::CallbackFn::new(move |_info, painter| {
                    // egui_glow has set the GL viewport to `rect` already.
                    screen.paint(painter.gl(), output.texture, FULL_CLIP, layer, filter, smooth);
                });
                ui.painter()
                    .add(egui::PaintCallback { rect, callback: std::sync::Arc::new(callback) });
            }
            return;
        }

        if let Some(textures) = &self.textures {
            paint_screens(ui.painter(), &placed, textures, self.view.rotation);
        }
    }
}
