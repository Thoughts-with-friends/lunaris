//! Putting the console's picture and its messages on screen.

use crate::app::*;

impl MelonEgui {
    /// The pointer's position on the bottom screen in touchscreen coordinates,
    /// or `None` when the stylus is not down on it.
    pub(crate) fn sample_touch(&self, ctx: &egui::Context) -> Option<(u16, u16)> {
        let rect = self.bottom_screen?;
        let pos =
            ctx.input(|i| i.pointer.primary_down().then(|| i.pointer.interact_pos()).flatten())?;
        touch_coords(rect, pos, self.view.rotation)
    }

    /// Copy both framebuffers into egui textures.
    ///
    /// Under an OpenGL renderer the picture never leaves the GPU and
    /// [`Self::screens`] draws it straight from the texture — except for the 2D
    /// round trip, which is [`Self::filter_gl_2d`].
    pub(crate) fn upload(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let filter =
            if self.view.filtering { TextureOptions::LINEAR } else { TextureOptions::NEAREST };
        let Some(emu) = &mut self.emu else {
            return;
        };
        if emu.gl_output().is_some() {
            self.filter_gl_2d(frame);
            // `ScreenSizing::Auto` decides on whether a screen has anything on
            // it, which cannot be sampled from a texture without reading it
            // back every frame. Both screens count as live instead, which is
            // what Auto resolves to whenever it cannot tell.
            self.screens_live = [true, true];
            return;
        }
        let Some((top, bottom)) = emu.nds.framebuffers() else {
            return;
        };
        // What `ScreenSizing::Auto` decides on: a screen showing nothing but
        // black is one the console is not really using.
        let lit = |fb: &[u32]| fb.iter().any(|&px| px & 0x00FF_FFFF != 0);
        let live = [lit(top), lit(bottom)];
        let (method, factor) = (self.video.upscale, self.video.upscale_factor());
        let images = [to_image(top, method, factor), to_image(bottom, method, factor)];
        self.screens_live = live;

        match &mut self.textures {
            // The options go in on every upload, so toggling `Screen filtering`
            // takes effect on the next frame without rebuilding the textures.
            Some(textures) => {
                for (texture, image) in textures.iter_mut().zip(images) {
                    texture.set(image, filter);
                }
            }
            None => {
                let [top, bottom] = images;
                self.textures = Some([
                    ctx.load_texture("ds-top", top, filter),
                    ctx.load_texture("ds-bottom", bottom, filter),
                ]);
            }
        }
    }

    /// Run the real xBRZ over the OpenGL renderer's 2D content.
    ///
    /// # Why the round trip
    ///
    /// The first attempt at this did the smoothing in the blit shader. It was
    /// free, but it was a *simplification* of xBRZ — one corner rule where the
    /// real filter reconstructs steep and shallow lines as well — and it
    /// showed: the picture changed, but nowhere near as much as the software
    /// renderer's.
    ///
    /// The real filter cannot run on the GPU, so the pixels come to it. Not the
    /// frame, though: only one texel per DS pixel, 256x192, which is where the
    /// 2D content actually lives (see [`crate::gl_screen::shader`]). That is
    /// 196 KB a screen and 49 152 pixels through xBRZ — the same work the
    /// software renderer already does — instead of 3 MB and 786 432 at 4x.
    ///
    /// The 3D never makes the trip and is never filtered: the shader shows the
    /// renderer's own texels wherever a DS pixel's block has detail inside it.
    /// So the internal resolution keeps every polygon it drew, and the 2D is
    /// put through exactly the filter the software renderer uses.
    fn filter_gl_2d(&mut self, frame: &eframe::Frame) {
        let Some(screen) = self.gl_screen.clone() else { return };
        let Some(gl) = frame.gl() else { return };
        let Some(output) = self.emu.as_mut().and_then(Emu::gl_output) else { return };

        if self.video.upscale == crate::upscale::Method::None || !screen.can_filter() {
            // Dropped rather than left behind, so switching the filter off
            // cannot leave one more frame of it on screen.
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

    // -- drawing ------------------------------------------------------------

    /// Lay the screens out in `area` and paint them, recording where the bottom
    /// one landed so the next repaint can map touch onto it.
    pub(crate) fn screens(&mut self, ui: &mut egui::Ui, area: Rect) {
        let placed = view::layout(area, &self.resolved_view());
        self.bottom_screen = placed.bottom;

        // Under OpenGL the picture is a texture in eframe's context rather than
        // CPU pixels, so it is drawn by a callback inside the GL painter.
        if let Some(output) = self.emu.as_mut().and_then(Emu::gl_output)
            && let Some(screen) = self.gl_screen.clone()
        {
            let filter =
                if self.view.filtering { eframe::glow::LINEAR } else { eframe::glow::NEAREST };
            // Show the xBRZ'd 2D where `filter_gl_2d` produced one; the
            // shader decides per fragment which pixels that is.
            let smooth = self.video.upscale == crate::upscale::Method::Xbrz;
            for (rect, layer) in [(placed.top, 0.0f32), (placed.bottom, 1.0f32)] {
                let Some(rect) = rect else { continue };
                let screen = screen.clone();
                let callback = egui_glow::CallbackFn::new(move |_info, painter| {
                    // egui_glow sets the GL viewport to this callback's own
                    // rectangle before calling it, so the quad just covers clip
                    // space and lands exactly where the layout put the screen.
                    screen.paint(painter.gl(), output.texture, FULL_CLIP, layer, filter, smooth);
                });
                ui.painter()
                    .add(egui::PaintCallback { rect, callback: std::sync::Arc::new(callback) });
            }
            return;
        }

        let Some(textures) = &self.textures else {
            return;
        };
        let painter = ui.painter();
        for (rect, texture) in [(placed.top, &textures[0]), (placed.bottom, &textures[1])] {
            if let Some(rect) = rect {
                paint_screen(painter, texture.id(), rect, self.view.rotation);
            }
        }
    }
}
