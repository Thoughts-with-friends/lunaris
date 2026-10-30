//! The on-screen display: what a command reports, drawn over the picture.
//!
//! melonDS puts its messages over the screens, so this front end does too.
//! The frame rate and the paused state live in the status bar
//! ([`crate::ui::status`]). The colour comes from the message's own
//! [`Severity`].

use crate::app::*;

impl MelonEgui {
    /// Draw the newest message (for [`OSD_LIFETIME`]).
    pub(crate) fn osd(&mut self, ui: &mut egui::Ui, area: Rect) {
        if !self.view.show_osd {
            return;
        }
        // Only the newest message, and only while it is fresh; it carries
        // whatever colour it earned.
        let mut lines = Vec::new();
        if let Some((notice, at)) = &self.osd {
            if at.elapsed() < OSD_LIFETIME {
                lines.push((notice.severity, notice.text.clone()));
            } else {
                self.osd = None;
            }
        }

        let painter = ui.painter();
        let mut at = area.left_top() + egui::vec2(6.0, 4.0);
        for (severity, line) in lines {
            // Drawn twice, offset, so the text stays readable over both a light
            // and a dark picture — the cheap equivalent of an outline. The OSD
            // sits over the console's own picture, so its `Info` is always the
            // light one whatever the window theme is.
            for (offset, color) in [(1.0, Color32::BLACK), (0.0, severity.color(true))] {
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
