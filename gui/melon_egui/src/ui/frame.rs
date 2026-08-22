//! One repaint, in the order it happens.
//!
//! Deliberately short and linear: this is the file to read first to see what a
//! frame of the front end actually does, and every step is one call into a
//! module named for it.

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

        // The core is paced off wall-clock time, so the window has to keep
        // repainting rather than wait for input. Paused, there is nothing to
        // redraw until something happens.
        // A dialog is answered on another thread, so the window has to keep
        // repainting to notice — that is what makes the console keep running
        // while it is open rather than freezing behind it.
        // A client repaints continuously: its picture arrives from the network
        // and its input has to leave on the same cadence, neither of which
        // egui knows to wake up for.
        if self.emu.is_some() && (!self.paused || self.step_pending)
            || self.mode == Mode::RemoteClient
            || self.lan_pending.is_some()
            || self.remote_pending.is_some()
            || self.dialog.is_some()
            || self.guest.is_some()
        {
            ctx.request_repaint();
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
