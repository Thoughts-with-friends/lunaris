//! The windows beside the main one: the second console, a second view of the
//! first, and the `--shot` capture.

use crate::app::*;

impl MelonEgui {
    /// Keys and touch for the second console, read from its own viewport.
    ///
    /// egui keeps a separate input state per viewport, so this reads the guest
    /// window's rather than the main window's — otherwise one keypress would
    /// drive both consoles.
    pub(crate) fn sample_guest_input(&self, ctx: &egui::Context) -> (u32, Option<(u16, u16)>) {
        if self.guest.is_none() {
            return (0, None);
        }
        let id = guest_viewport_id();
        let read = |i: &egui::InputState| {
            let keys = self.bindings.key_mask(i);
            let pointer = i.pointer.primary_down().then(|| i.pointer.interact_pos()).flatten();
            (keys, pointer)
        };
        // Before the viewport's first repaint this reads a default state, which
        // is simply "nothing held" -- the right answer for a window that has
        // not appeared yet.
        let (keys, pointer) = ctx.input_for(id, read);
        let touch = self
            .guest_bottom
            .zip(pointer)
            .and_then(|(rect, pos)| touch_coords(rect, pos, self.instance2_settings.view.rotation));
        (keys, touch)
    }

    /// The second console's window: its own screens, its own input.
    pub(crate) fn guest_view(&mut self, ctx: &egui::Context) {
        if self.guest.is_none() {
            return;
        }
        let host_settings = self.settings();
        self.apply_runtime_settings(&self.instance2_settings.clone(), 2);
        // Upload the guest's picture with the same conversion the host uses.
        let filter =
            if self.view.filtering { TextureOptions::LINEAR } else { TextureOptions::NEAREST };
        if let Some(screens) = self.guest.as_ref().and_then(crate::guest::Guest::take_screens) {
            let [top, bottom] = screens;
            let images = [
                to_image(&top, self.video.upscale, self.video.upscale_factor()),
                to_image(&bottom, self.video.upscale, self.video.upscale_factor()),
            ];
            match &mut self.guest_textures {
                Some(textures) => {
                    for (texture, image) in textures.iter_mut().zip(images) {
                        texture.set(image, filter);
                    }
                }
                None => {
                    let [t, b] = images;
                    self.guest_textures = Some([
                        ctx.load_texture("guest-top", t, filter),
                        ctx.load_texture("guest-bottom", b, filter),
                    ]);
                }
            }
        }

        let Some(textures) = self.guest_textures.clone() else {
            let updated = self.settings();
            updated.save_for(2);
            self.instance2_settings = updated;
            self.apply_runtime_settings(&host_settings, 1);
            return;
        };
        let view = self.resolved_view();
        let builder = egui::ViewportBuilder::default()
            .with_title("melon_egui - instance 2")
            .with_inner_size(default_window_size())
            // Same COM-apartment reason as the main window.
            .with_drag_and_drop(false);

        let mut closed = false;
        let mut bottom_rect = None;
        let mut action = None;
        ctx.show_viewport_immediate(guest_viewport_id(), builder, |ctx, _class| {
            ctx.set_zoom_factor(self.ui_scale);
            self.set_theme(ctx, self.dark_theme);
            egui::TopBottomPanel::top("guest-menu").show(ctx, |ui| {
                action = menu::bar(self, ui);
            });
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(
                ctx,
                |ui| {
                    let area = ui.max_rect();
                    let placed = view::layout(area, &view);
                    bottom_rect = placed.bottom;
                    let painter = ui.painter();
                    for (rect, texture) in
                        [(placed.top, &textures[0]), (placed.bottom, &textures[1])]
                    {
                        if let Some(rect) = rect {
                            paint_screen(painter, texture.id(), rect, view.rotation);
                        }
                    }
                },
            );
            panes::show(self, ctx);
            // Resizing has to happen against *this* viewport's context, so it
            // is taken here rather than in `apply_to_guest`: sending it to the
            // main window's context would resize the wrong window.
            if let Some(Action::ScreenSize(scale)) = action {
                let size = view::window_size_for_scale(scale, &self.view, CHROME_HEIGHT);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                action = None;
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                closed = true;
            }
        });
        self.guest_bottom = bottom_rect;
        let updated = self.settings();
        updated.save_for(2);
        self.instance2_settings = updated;
        self.apply_runtime_settings(&host_settings, 1);
        ctx.set_zoom_factor(self.ui_scale);
        self.set_theme(ctx, self.dark_theme);
        // Routed to the *second* console. Before this existed the second
        // window's menu bar drove the first console, which is what "only some
        // of it works over there" was.
        if let Some(action) = action {
            self.apply_to_guest(action);
        }
        if closed {
            self.guest = None;
            self.guest_textures = None;
        }
    }

    /// A second window showing the same console, as melonDS's "Open new window"
    /// does. It shares the textures, so it costs a blit and no emulation.
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
            .with_title("melon_egui - second view")
            .with_inner_size(default_window_size())
            // Same reason main.rs needs it: winit's drag-and-drop support
            // initialises COM as an STA, which conflicts with an MTA already
            // established on this process.
            .with_drag_and_drop(false);

        let mut closed = false;
        ctx.show_viewport_immediate(id, builder, |ctx, _class| {
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(
                ctx,
                |ui| {
                    let area = ui.max_rect();
                    let placed = view::layout(area, &view);
                    let painter = ui.painter();
                    for (rect, texture) in
                        [(placed.top, &textures[0]), (placed.bottom, &textures[1])]
                    {
                        if let Some(rect) = rect {
                            paint_screen(painter, texture.id(), rect, view.rotation);
                        }
                    }
                },
            );
            if ctx.input(|i| i.viewport().close_requested()) {
                closed = true;
            }
        });
        if closed {
            self.second_window = false;
        }
    }

    /// Drive a pending `--shot`: ask for the capture once the cart has run far
    /// enough, then write whatever egui hands back and quit.
    ///
    /// The image arrives on a later repaint as an [`egui::Event::Screenshot`],
    /// because the frame has to reach the GPU before it can be read back.
    pub(crate) fn service_shot(&mut self, ctx: &egui::Context) {
        let Some((at, path)) = &self.shot else {
            return;
        };

        if let Some(image) = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(std::sync::Arc::clone(image)),
                _ => None,
            })
        }) {
            let rgba: Vec<u8> = image.pixels.iter().flat_map(Color32::to_array).collect();
            let [w, h] = image.size;
            let result = image::save_buffer(
                path,
                &rgba,
                w as u32,
                h as u32,
                image::ExtendedColorType::Rgba8,
            );
            match result {
                Ok(()) => log::info!("shot: wrote {} ({w}x{h})", path.display()),
                Err(e) => log::error!("shot: failed to write {}: {e}", path.display()),
            }
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

    /// Alongside a `--shot` of the window, write the core's own picture when it
    /// is an OpenGL renderer drawing it: `<out>_core_top.png` and
    /// `<out>_core_bottom.png`, read back from the texture at the internal
    /// resolution.
    ///
    /// The window capture is at window size whatever the renderer is doing, so
    /// it cannot show that the internal resolution reached the rasteriser.
    /// These can: their pixel size *is* `256*scale x 192*scale`.
    pub(crate) fn shot_core_picture(&mut self, path: PathBuf) {
        let Some(emu) = &mut self.emu else { return };
        let Some(output) = emu.gl_output() else { return };

        let (w, h) = (output.width as usize, output.height as usize);
        let mut pixels = vec![0u32; w * h];
        for (screen, name) in [(0u8, "top"), (1, "bottom")] {
            if emu.gl_read_output(screen, &mut pixels) == 0 {
                log::error!("shot: could not read the {name} screen back from the GL renderer");
                continue;
            }
            // BGRA in memory, as the software framebuffers are, so the channel
            // order here is the one `to_image` uses.
            let rgb: Vec<u8> = pixels
                .iter()
                .flat_map(|&px| [(px >> 16) as u8, (px >> 8) as u8, px as u8])
                .collect();
            let out = path.with_file_name(format!(
                "{}_core_{name}.png",
                path.file_stem().unwrap_or_default().to_string_lossy()
            ));
            match image::save_buffer(&out, &rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8)
            {
                Ok(()) => log::info!("shot: wrote {} ({w}x{h})", out.display()),
                Err(e) => log::error!("shot: failed to write {}: {e}", out.display()),
            }
        }
    }
    pub(crate) fn update_window_info(&mut self, ctx: &egui::Context) {
        update_window_geometry(ctx, egui::ViewportId::ROOT, &mut self.window);
    }
}

fn update_window_geometry(
    ctx: &egui::Context,
    viewport_id: egui::ViewportId,
    geometry: &mut WindowConfig,
) {
    // NOTE: Writing directly to `geometry` inside this closure would
    // deadlock (egui holds an internal lock during `input()`).
    let (pos, size, maximized) = ctx.input(|i| {
        let mut temp_pos = None;
        let mut temp_size = None;
        let mut temp_maximized = None;

        if let Some(info) = i.raw.viewports.get(&viewport_id) {
            temp_maximized = Some(info.maximized.unwrap_or(false));

            if let Some(inner_rect) = info.inner_rect {
                temp_size = Some(inner_rect.size());
            }

            if let Some(outer_rect) = info.outer_rect {
                temp_pos = Some(outer_rect.min);
            }
        }

        (temp_pos, temp_size, temp_maximized)
    });

    if !geometry.maximized {
        if let Some(pos) = pos {
            geometry.pos_x = pos.x;
            geometry.pos_y = pos.y;
        }

        if let Some(size) = size {
            geometry.width = size.x;
            geometry.height = size.y;
        }
    }

    if let Some(maximized) = maximized {
        geometry.maximized = maximized;
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    /// X coordinate of the window's top-left corner (outer rect).
    pub pos_x: f32,

    /// Y coordinate of the window's top-left corner (outer rect).
    pub pos_y: f32,

    /// Inner width of the window (excludes OS decorations).
    pub width: f32,

    /// Inner height of the window (excludes title bar and OS decorations).
    pub height: f32,

    /// Whether the window was maximized when the application last closed.
    pub maximized: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { pos_x: 100.0, pos_y: 100.0, width: 512.0, height: 768.0, maximized: false }
    }
}
