//! The windows beside the main one, the `--shot` capture, and remembering the
//! main window's position.
//!
//! # The second console's window ([`MelonEgui::guest_view`])
//!
//! It has its own `instance2/settings.json` (view, language, theme...), yet it
//! is drawn by the same `MelonEgui`. So each repaint:
//!
//! ```text
//!  1. swap instance 2's settings in       (apply_runtime_settings(.., 2))
//!  2. upload its newest picture
//!  3. draw its viewport: menu bar, screens, panes; read its close button
//!  4. save instance 2's settings if they changed, swap instance 1's back
//!  5. act on its menu click                (apply_to_guest)
//! ```

use crate::app::*;

impl MelonEgui {
    /// Keys and touch for the second console, read from *its* viewport's input
    /// (egui keeps one per viewport, so one keypress cannot drive both).
    pub(crate) fn sample_guest_input(&self, ctx: &egui::Context) -> (u32, Option<(u16, u16)>) {
        if self.guest.is_none() {
            return (0, None);
        }
        let read = |i: &egui::InputState| {
            let keys = self.bindings.key_mask(i);
            let pointer = i.pointer.primary_down().then(|| i.pointer.interact_pos()).flatten();
            (keys, pointer)
        };
        // Before the window first appears this reads "nothing held".
        let (keys, pointer) = ctx.input_for(guest_viewport_id(), read);
        let touch = self
            .guest_bottom
            .zip(pointer)
            .and_then(|(rect, pos)| touch_coords(rect, pos, self.instance2_settings.view.rotation));
        (keys, touch)
    }

    /// The second console's window: its own screens, menu, panes and input.
    pub(crate) fn guest_view(&mut self, ctx: &egui::Context) {
        if self.guest.is_none() {
            return;
        }
        let host_settings = self.settings();
        self.apply_runtime_settings(&self.instance2_settings.clone(), 2);
        if let Some([top, bottom]) = self.guest.as_ref().and_then(crate::guest::Guest::take_screens)
        {
            let (video, view) = (self.video, self.view);
            upload_screens(
                ctx,
                &mut self.guest_textures,
                ["guest-top", "guest-bottom"],
                [&top, &bottom],
                &video,
                &view,
            );
        }
        let shown =
            self.guest_textures.clone().map(|textures| self.show_guest_window(ctx, textures));
        self.store_guest_settings();
        self.apply_runtime_settings(&host_settings, 1);

        let Some((action, closed)) = shown else { return };
        ctx.set_zoom_factor(self.ui_scale);
        self.set_theme(ctx, self.dark_theme);
        // Routed to the *second* console (it used to drive the first one).
        if let Some(action) = action {
            self.apply_to_guest(action);
        }
        if closed {
            self.close_guest();
        }
    }

    /// Draw the second console's viewport. Returns its menu click (if any) and
    /// whether its close button was pressed.
    fn show_guest_window(
        &mut self,
        ctx: &egui::Context,
        textures: [TextureHandle; 2],
    ) -> (Option<Action>, bool) {
        let view = self.resolved_view();
        let builder = egui::ViewportBuilder::default()
            .with_title(self.i18n().t(crate::i18n::I18nKey::Instance2Title))
            .with_inner_size(default_window_size())
            // winit's drag-and-drop initialises COM as STA, which conflicts with
            // the audio thread's MTA (see `crate::audio`).
            .with_drag_and_drop(false);

        let (mut action, mut closed) = (None, false);
        ctx.show_viewport_immediate(guest_viewport_id(), builder, |ctx, _class| {
            ctx.set_zoom_factor(self.ui_scale);
            self.set_theme(ctx, self.dark_theme);
            egui::TopBottomPanel::top("guest-menu").show(ctx, |ui| action = menu::bar(self, ui));
            self.guest_bottom = screen_panel(ctx, &view, &textures).bottom;
            panes::show(self, ctx);
            // Resizing must use *this* viewport's context, or the wrong window
            // would resize.
            if let Some(Action::ScreenSize(scale)) = action {
                self.resize_for_scale(ctx, scale);
                action = None;
            }
            closed = ctx.input(|i| i.viewport().close_requested());
        });
        (action, closed)
    }

    /// Write `instance2/settings.json` when what it would hold has changed
    /// (and once when the window opens), rather than on every repaint.
    fn store_guest_settings(&mut self) {
        let updated = self.settings();
        let json = serde_json::to_string(&updated).unwrap_or_default();
        if json != self.instance2_saved {
            updated.save_for(2);
            self.instance2_saved = json;
        }
        self.instance2_settings = updated;
    }

    /// A second window showing the *same* console (melonDS's "Open new
    /// window"). It shares the textures, so it costs a blit and no emulation.
    pub(crate) fn second_view(&mut self, ctx: &egui::Context) {
        if !self.second_window {
            return;
        }
        let Some(textures) = self.textures.clone() else {
            return;
        };
        let view = self.resolved_view();
        let id = egui::ViewportId::from_hash_of("melon_egui-second-view");
        let builder = egui::ViewportBuilder::default()
            .with_title(self.i18n().t(crate::i18n::I18nKey::SecondViewTitle))
            .with_inner_size(default_window_size())
            .with_drag_and_drop(false);

        let mut closed = false;
        ctx.show_viewport_immediate(id, builder, |ctx, _class| {
            screen_panel(ctx, &view, &textures);
            closed = ctx.input(|i| i.viewport().close_requested());
        });
        if closed {
            self.second_window = false;
        }
    }

    /// Drive a pending `--shot`: once enough frames have run, ask egui for a
    /// screenshot; when it arrives (a later repaint), write it and quit.
    pub(crate) fn service_shot(&mut self, ctx: &egui::Context) {
        let Some((at, path)) = &self.shot else {
            return;
        };

        let image = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(std::sync::Arc::clone(image)),
                _ => None,
            })
        });
        if let Some(image) = image {
            let rgba: Vec<u8> = image.pixels.iter().flat_map(Color32::to_array).collect();
            let [w, h] = image.size;
            save_png(path, &rgba, w, h, image::ExtendedColorType::Rgba8);
            self.shot_core_picture(path.clone());
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if !self.shot_requested && self.frames_run >= *at {
            self.shot_requested = true;
            log::info!("shot: {} frames run, requesting capture", self.frames_run);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }

    /// Under an OpenGL renderer, also write the core's own texture at its
    /// internal resolution (`<out>_core_top.png`, `<out>_core_bottom.png`):
    /// the window capture cannot show that the internal resolution was used.
    pub(crate) fn shot_core_picture(&mut self, path: PathBuf) {
        let Some(emu) = &mut self.emu else { return };
        let Some(output) = emu.nds.gl_output() else { return };

        let (w, h) = (output.width as usize, output.height as usize);
        let mut pixels = vec![0u32; w * h];
        for (screen, name) in [(0u8, "top"), (1, "bottom")] {
            if emu.nds.gl_read_output(screen, &mut pixels) == 0 {
                log::error!("shot: could not read the {name} screen back from the GL renderer");
                continue;
            }
            // BGRA in memory, like the software framebuffers.
            let rgb: Vec<u8> = pixels
                .iter()
                .flat_map(|&px| [(px >> 16) as u8, (px >> 8) as u8, px as u8])
                .collect();
            let out = path.with_file_name(format!(
                "{}_core_{name}.png",
                path.file_stem().unwrap_or_default().to_string_lossy()
            ));
            save_png(&out, &rgb, w, h, image::ExtendedColorType::Rgb8);
        }
    }

    /// Remember the main window's position and size for the next run.
    pub(crate) fn update_window_info(&mut self, ctx: &egui::Context) {
        // Read inside `input`, written outside it: writing while egui holds its
        // input lock would deadlock.
        let info = ctx.input(|i| {
            i.raw.viewports.get(&egui::ViewportId::ROOT).map(|info| {
                (
                    info.outer_rect.map(|rect| rect.min),
                    info.inner_rect.map(|rect| rect.size()),
                    info.maximized.unwrap_or(false),
                )
            })
        });
        let Some((pos, size, maximized)) = info else { return };
        let geometry = &mut self.window;
        // A maximised window's rectangle is not the one to restore to.
        if !geometry.maximized {
            if let Some(pos) = pos {
                (geometry.pos_x, geometry.pos_y) = (pos.x, pos.y);
            }
            if let Some(size) = size {
                (geometry.width, geometry.height) = (size.x, size.y);
            }
        }
        geometry.maximized = maximized;
    }
}

/// Write a PNG for `--shot`, logging the outcome either way.
fn save_png(path: &Path, bytes: &[u8], w: usize, h: usize, colour: image::ExtendedColorType) {
    match image::save_buffer(path, bytes, w as u32, h as u32, colour) {
        Ok(()) => log::info!("shot: wrote {} ({w}x{h})", path.display()),
        Err(e) => log::error!("shot: failed to write {}: {e}", path.display()),
    }
}

/// The main window's last position and size, saved in `settings.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    /// Left edge of the outer window (including OS decorations).
    pub pos_x: f32,
    /// Top edge of the outer window.
    pub pos_y: f32,
    /// Inner width (excluding OS decorations).
    pub width: f32,
    /// Inner height (excluding the title bar).
    pub height: f32,
    /// Whether the window was maximised when it closed.
    pub maximized: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { pos_x: 100.0, pos_y: 100.0, width: 512.0, height: 768.0, maximized: false }
    }
}
