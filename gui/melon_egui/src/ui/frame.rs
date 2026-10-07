//! One repaint, in the order it happens — the place to start reading.
//!
//! ```text
//!  update()
//!   1. advance            run the console, upload its picture   (app/emulation.rs)
//!   2. update_window_info remember the window's position        (ui/window.rs)
//!   3. menu bar           → Option<Action>                      (ui/menu)
//!   4. central panel      screens + OSD                          (ui/screen.rs, ui/osd.rs)
//!   5. panes::show        the open settings windows             (ui/panes)
//!   6. guest_view         the second console's window           (ui/window.rs)
//!   7. second_view        a second view of the first console    (ui/window.rs)
//!   8. apply(action)      do what the menu asked                (app/commands.rs)
//!   9. service_shot       --shot capture, when due               (ui/window.rs)
//!  10. request_repaint    if anything is running or pending
//! ```

use crate::app::*;

impl eframe::App for MelonEgui {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // `frame` carries eframe's GL context, which is what lets the 2D
        // round trip in `crate::gl_screen::capture` happen at all: it needs
        // the context current, and a paint callback is too late.
        self.advance(ctx, frame);
        self.update_window_info(ctx);

        let mut action = None;
        egui::TopBottomPanel::top("menu").show(ctx, |ui| action = menu::bar(self, ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(
            ctx,
            |ui| {
                let area = ui.max_rect();
                self.screens(ui, area);
                self.osd(ui, area);
            },
        );
        panes::show(self, ctx);
        self.guest_view(ctx);
        self.second_view(ctx);
        if let Some(action) = action {
            self.apply(action, ctx);
        }

        self.service_shot(ctx);

        // egui only repaints on input by default. Keep repainting while a
        // console runs (it is paced by the clock), while a client streams,
        // and while a worker thread or file dialog may answer — none of which
        // egui would wake up for. Paused and idle, the window sleeps.
        if self.emu.is_some() && (!self.paused || self.step_pending)
            || self.mode == Mode::RemoteClient
            || self.lan_pending.is_some()
            || self.remote_pending.is_some()
            || self.dialog.is_some()
            || self.guest.is_some()
        {
            ctx.request_repaint();
        } else if self.remote_probe.is_some() {
            // The partner's readiness changes with nobody touching the window:
            // look again at the rate it is probed, so red turns green on its own.
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Some(emu) = &self.emu {
            emu.flush_save();
        }
        // The blitter's program and vertex array belong to eframe's context,
        // which is still current here and gone afterwards.
        if let (Some(gl), Some(screen)) = (gl, self.gl_screen.take()) {
            screen.destroy(gl);
        }
        self.persist();
    }
}
