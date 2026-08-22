//! Putting the console's picture and its messages on screen.

use super::*;

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
    /// Nothing to do under an OpenGL renderer: its picture never leaves the
    /// GPU, and is drawn by [`Self::screens`] straight from the texture.
    pub(crate) fn upload(&mut self, ctx: &egui::Context) {
        let filter =
            if self.view.filtering { TextureOptions::LINEAR } else { TextureOptions::NEAREST };
        let Some(emu) = &mut self.emu else {
            return;
        };
        if emu.gl_output().is_some() {
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
            for (rect, layer) in [(placed.top, 0.0f32), (placed.bottom, 1.0f32)] {
                let Some(rect) = rect else { continue };
                let screen = screen.clone();
                let callback = egui_glow::CallbackFn::new(move |_info, painter| {
                    // egui_glow sets the GL viewport to this callback's own
                    // rectangle before calling it, so the quad just covers clip
                    // space and lands exactly where the layout put the screen.
                    screen.paint(painter.gl(), output.texture, FULL_CLIP, layer, filter);
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

    /// The OSD: melonDS draws its messages and its frame rate over the picture
    /// rather than in a status bar, so this front end does too.
    pub(crate) fn osd(&mut self, ui: &mut egui::Ui, area: Rect) {
        if !self.view.show_osd {
            return;
        }
        // Only the newest message, and only while it is fresh.
        let mut lines = Vec::new();
        if let Some((message, at)) = &self.osd {
            if at.elapsed() < OSD_LIFETIME {
                lines.push(message.clone());
            } else {
                self.osd = None;
            }
        }
        if self.is_loaded() {
            let paused = if self.paused { "  [paused]" } else { "" };
            // Without this the window looks hung rather than deliberately still.
            let frozen = if self.video.render { "" } else { "  [rendering off]" };
            lines.insert(0, format!("{:.1} FPS{paused}{frozen}", self.fps));
        }

        let painter = ui.painter();
        let mut at = area.left_top() + egui::vec2(6.0, 4.0);
        for line in lines {
            // Drawn twice, offset, so the text stays readable over both a light
            // and a dark picture — the cheap equivalent of an outline.
            for (offset, color) in [(1.0, Color32::BLACK), (0.0, Color32::WHITE)] {
                painter.text(
                    at + egui::vec2(offset, offset),
                    egui::Align2::LEFT_TOP,
                    &line,
                    egui::FontId::monospace(13.0),
                    color,
                );
            }
            at.y += 16.0;
        }
    }
}
