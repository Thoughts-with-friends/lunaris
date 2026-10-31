//! The status bar at the bottom of the window, laid out as the freya GUI's: every item
//! is a chip that changes what it shows, or opens where it is set.
//!
//! ```text
//!  ● Running · Pause  ⇄ Remote…  game.nds ……  59.8 fps  ♪ 100%  1x  1024×768  xBRZ 2x  ⚙ melonDS  instance1
//!   pause / resume    Wireless   ROM info     limiter   Audio   speed  internal  off,2..6x Emu       folder
//!                                                                      res 1..16x
//! ```
//!
//! What a click asks of the app is returned as an [`Action`] like a menu
//! entry's; the settings it only toggles (speed, limiter, internal
//! resolution, xBRZ) are changed in place.

use egui::{Color32, RichText, Ui};

use crate::{
    app::{MelonEgui, Pane},
    i18n::I18nKey as K,
    ui::menu::Action,
    upscale,
    video::Renderer,
};

/// Height of the bar, in points (part of [`crate::app::CHROME_HEIGHT`]).
pub const STATUS_HEIGHT: f32 = 26.0;

const SUCCESS: Color32 = Color32::from_rgb(96, 214, 150);
const WARNING: Color32 = Color32::from_rgb(255, 196, 90);
const ACCENT: Color32 = Color32::from_rgb(124, 140, 255);
const ACCENT_2: Color32 = Color32::from_rgb(198, 120, 255);
const FAINT: Color32 = Color32::from_rgb(105, 110, 135);

/// Highest internal resolution the chip steps to (the core's own limit).
const MAX_INTERNAL_SCALE: u32 = 16;

/// "0.5x", "1x", "1.5x", ...
fn speed_label(speed: f32) -> String {
    format!("{speed}x")
}

/// The internal resolution after `scale`, wrapping 16x back to 1x.
const fn next_internal_scale(scale: u32) -> u32 {
    scale % MAX_INTERNAL_SCALE + 1
}

/// xBRZ off → 2x → … → 6x → off.
fn next_xbrz(method: upscale::Method, factor: u8) -> (upscale::Method, u8) {
    match method {
        upscale::Method::None => (upscale::Method::Xbrz, 2),
        upscale::Method::Xbrz if factor < upscale::MAX_FACTOR => {
            (upscale::Method::Xbrz, (factor + 1).max(2))
        }
        upscale::Method::Xbrz => (upscale::Method::None, factor),
    }
}

/// A tinted, rounded button.
fn chip(ui: &mut Ui, text: impl Into<String>, color: Color32) -> egui::Response {
    let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 38);
    ui.add(
        egui::Button::new(RichText::new(text).size(11.5).color(color))
            .fill(fill)
            .stroke(egui::Stroke::NONE)
            .corner_radius(9.0),
    )
}

impl MelonEgui {
    /// Draw the bar, returning whatever a chip asked for.
    pub(crate) fn status_bar(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            action = self.status_left(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Right to left: the last chip first.
                let right = self.status_right(ui);
                action = action.take().or(right);
            });
        });
        action
    }

    /// The state, a Remote Desktop session, and the game.
    fn status_left(&self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        let tr = self.translations.get(self.language);
        let loaded = self.is_loaded();
        let (state, color) = match (loaded, self.paused) {
            (true, true) => (tr.t(K::StatusPaused), WARNING),
            (true, false) => (tr.t(K::StatusRunning), SUCCESS),
            // A client shows a console running somewhere else.
            (false, _) if self.mode == crate::app::Mode::RemoteClient => {
                (tr.t(K::StatusRemoteScreen), SUCCESS)
            }
            (false, _) => (tr.t(K::StatusIdle), FAINT),
        };
        if chip(ui, format!("● {state}"), color).clicked() {
            action = Some(if loaded { Action::TogglePause } else { Action::OpenRom });
        }

        let remote = match (&self.remote_host, &self.remote_client) {
            (Some(host), _) => Some((tr.f(K::StatusRemoteClient, &[&host.remote_addr()]), SUCCESS)),
            (_, Some(client)) => {
                Some((tr.f(K::StatusRemoteHost, &[&client.remote_addr()]), SUCCESS))
            }
            _ if self.remote_pending.is_some() => Some((
                tr.s(if self.remote_pending_host {
                    K::StatusRemoteWaiting
                } else {
                    K::StatusRemoteConnecting
                }),
                WARNING,
            )),
            _ => None,
        };
        if let Some((text, color)) = remote
            && chip(ui, text, color).clicked()
        {
            action = Some(Action::TogglePane(Pane::Wireless));
        }

        let name = if loaded { self.cart_label() } else { tr.s(K::StatusNoGame) };
        let game = ui.add(
            egui::Label::new(RichText::new(name).size(11.5)).truncate().sense(egui::Sense::click()),
        );
        if game.clicked() {
            action = Some(if loaded { Action::TogglePane(Pane::RomInfo) } else { Action::OpenRom });
        }
        action
    }

    /// fps, volume, speed, internal resolution, xBRZ, core, instance — laid
    /// out right to left, so in reverse.
    fn status_right(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        let mut changed = false;

        if chip(ui, "instance1", FAINT).clicked() {
            action = Some(Action::OpenDirectory);
        }
        if chip(ui, crate::backend::CORE_CHIP, ACCENT_2).clicked() {
            action = Some(Action::TogglePane(Pane::EmuSettings));
        }

        // xBRZ (off, 2x .. 6x): the 2D under either renderer.
        let video = self.video;
        let (text, color) = match video.upscale {
            upscale::Method::Xbrz => (format!("xBRZ {}x", video.upscale_factor()), ACCENT_2),
            upscale::Method::None => (self.i18n().s(K::StatusXbrzOff), FAINT),
        };
        if chip(ui, text, color).clicked() {
            (self.video.upscale, self.video.upscale_factor) =
                next_xbrz(video.upscale, video.upscale_factor());
            changed = true;
        }

        // The OpenGL renderer's internal resolution (1x .. 16x): the 3D gains
        // detail. The software renderer has none, so a step above 1x moves to
        // OpenGL where it can.
        let gl = video.renderer.is_gl();
        let scale = if gl { video.scale() } else { 1 };
        let res = format!(
            "{}×{}",
            crate::app::SCREEN_WIDTH * scale as usize,
            crate::app::SCREEN_HEIGHT * scale as usize
        );
        let gl_ok = self.gl_available();
        let res = ui.add_enabled_ui(gl_ok, |ui| chip(ui, res, ACCENT_2)).inner;
        if res.clicked() {
            let next = next_internal_scale(scale);
            self.video.internal_scale = next;
            if !gl && next > 1 {
                self.video.renderer = Renderer::OpenGl;
            }
            changed = true;
        }

        // Real time while a second console or a LAN link needs it.
        let speed = self.effective_speed();
        let speed = ui
            .add_enabled_ui(!self.speed_locked(), |ui| chip(ui, speed_label(speed), ACCENT))
            .inner;
        if speed.clicked() {
            self.cycle_speed();
            changed = true;
        }

        let volume = (self.volume() * 100.0).round() as u32;
        let (text, color) = if volume == 0 {
            (format!("♪ {}", self.i18n().s(K::StatusMuted)), FAINT)
        } else {
            (format!("♪ {volume}%"), ACCENT)
        };
        if chip(ui, text, color).clicked() {
            action = Some(Action::TogglePane(Pane::AudioSettings));
        }

        // View > Show FPS; a click toggles the framerate limiter.
        if self.is_loaded() && self.view.show_fps {
            let tr = self.translations.get(self.language);
            let mut text = format!("{:.1} fps", self.fps);
            if !self.limit_framerate {
                text = format!("{text} · {}", tr.t(K::StatusUnlimited));
            }
            // Without this the window looks hung rather than deliberately still.
            if !self.video.render {
                text = format!("{text} {}", tr.t(K::OsdRenderingOff));
            }
            if chip(ui, text, SUCCESS).clicked() {
                self.limit_framerate ^= true;
                changed = true;
            }
        }

        if changed {
            self.persist();
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_resolution_steps_to_16x_and_wraps() {
        assert_eq!(next_internal_scale(1), 2);
        assert_eq!(next_internal_scale(15), 16);
        assert_eq!(next_internal_scale(16), 1);
    }

    #[test]
    fn xbrz_steps_off_2x_to_6x_off() {
        use upscale::Method::{None, Xbrz};
        let mut state = (None, 2);
        let mut seen = Vec::new();
        for _ in 0..6 {
            state = next_xbrz(state.0, state.1);
            seen.push(if state.0 == Xbrz { state.1 } else { 0 });
        }
        assert_eq!(seen, [2, 3, 4, 5, 6, 0]);
    }

    #[test]
    fn speeds_read_as_freya_writes_them() {
        assert_eq!(speed_label(0.5), "0.5x");
        assert_eq!(speed_label(1.0), "1x");
        assert_eq!(speed_label(1.5), "1.5x");
    }
}
