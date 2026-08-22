//! The on-screen display: what a command reports, drawn over the picture.
//!
//! melonDS puts its messages and its frame rate over the screens rather than in
//! a status bar, so this front end does too. The colour comes from the
//! message's own [`Severity`](crate::ui::notice::Severity).

use crate::app::*;

impl MelonEgui {
    /// The OSD: melonDS draws its messages and its frame rate over the picture
    /// rather than in a status bar, so this front end does too.
    pub(crate) fn osd(&mut self, ui: &mut egui::Ui, area: Rect) {
        if !self.view.show_osd {
            return;
        }
        // Only the newest message, and only while it is fresh. The readout is
        // always neutral; the message carries whatever colour it earned.
        let mut lines = Vec::new();
        if let Some((notice, at)) = &self.osd {
            if at.elapsed() < OSD_LIFETIME {
                lines.push((notice.severity, notice.text.clone()));
            } else {
                self.osd = None;
            }
        }
        if self.is_loaded() {
            let paused = if self.paused { "  [paused]" } else { "" };
            // Without this the window looks hung rather than deliberately still.
            let frozen = if self.video.render { "" } else { "  [rendering off]" };
            lines.insert(0, (Severity::Info, format!("{:.1} FPS{paused}{frozen}", self.fps)));
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
