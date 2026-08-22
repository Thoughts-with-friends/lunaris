//! The auxiliary windows behind the menu's dialog entries.
//!
//! melonDS opens each of these as a modal Qt dialog; here they are ordinary
//! egui windows, so several can be open at once and none of them blocks
//! emulation.

use egui::Context;

use crate::{app::MelonEgui, config, mp::Kind, upscale, video::Renderer, view::AspectRatio};

mod cheat_codes;
mod console;
mod interface;
mod paths;
mod ram_search;
mod remote;
mod settings;
mod wireless;

use cheat_codes::*;
use console::*;
use interface::*;
pub use paths::PathSetting;
use paths::*;
use ram_search::*;
pub use ram_search::{RamSearch, SearchWidth};
use settings::*;
use wireless::*;

/// One auxiliary window.
///
/// Serialisable so that whichever dialogs were open are reopened next run, the
/// way a docked tool window would be.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Pane {
    RomInfo,
    Power,
    Cheats,
    Crash,
    RamSearch,
    DateTime,
    Input,
    EmuSettings,
    Preferences,
    VideoSettings,
    AudioSettings,
    Wireless,
    Interface,
    Paths,
    About,
}

impl Pane {
    /// The window title, which is also its egui identity.
    pub const fn title(self) -> &'static str {
        match self {
            Self::RomInfo => "ROM info",
            Self::Power => "Power management",
            Self::Cheats => "Cheat codes",
            Self::Crash => "Why the console stopped",
            Self::RamSearch => "RAM search",
            Self::DateTime => "Date and time",
            Self::Input => "Input and hotkeys",
            Self::EmuSettings => "Emu settings",
            Self::Preferences => "Preferences",
            Self::VideoSettings => "Video settings",
            Self::AudioSettings => "Audio settings",
            Self::Wireless => "Wireless status",
            Self::Interface => "Interface settings",
            Self::Paths => "Path settings",
            Self::About => "About melon_egui",
        }
    }
}

/// Draw every open pane, closing any whose window was dismissed.
pub fn show(app: &mut MelonEgui, ctx: &Context) {
    for pane in app.open_panes() {
        let mut open = true;
        egui::Window::new(pane.title())
            .open(&mut open)
            .resizable(matches!(
                pane,
                Pane::RamSearch | Pane::Wireless | Pane::Cheats | Pane::Crash | Pane::Input
            ))
            // The two that are wider by nature: the wireless dialog is a table
            // of counters, and Input is three columns of bindings.
            .default_width(if matches!(pane, Pane::Wireless | Pane::Input) { 460.0 } else { 260.0 })
            .show(ctx, |ui| body(app, pane, ui));
        if !open {
            app.close_pane(pane);
        }
    }
}

fn body(app: &mut MelonEgui, pane: Pane, ui: &mut egui::Ui) {
    match pane {
        Pane::RomInfo => rom_info(app, ui),
        Pane::Power => power(app, ui),
        Pane::Cheats => cheat_codes(app, ui),
        Pane::Crash => crash(app, ui),
        Pane::RamSearch => ram_search(app, ui),
        Pane::DateTime => date_time(app, ui),
        Pane::Input => input(app, ui),
        Pane::EmuSettings => emu_settings(app, ui),
        Pane::Preferences => preferences(app, ui),
        Pane::VideoSettings => video_settings(app, ui),
        Pane::AudioSettings => audio_settings(app, ui),
        Pane::Wireless => wireless(app, ui),
        Pane::Interface => interface(app, ui),
        Pane::Paths => paths(app, ui),
        Pane::About => about(ui),
    }
}

/// A checkbox present for shape but not usable, with the reason on hover.
fn disabled_checkbox(ui: &mut egui::Ui, label: &str, why: &str) {
    let mut off = false;
    ui.add_enabled(false, egui::Checkbox::new(&mut off, label)).on_disabled_hover_text(why);
}

#[cfg(test)]
mod tests {
    use super::{RamSearch, SearchWidth};

    #[test]
    fn the_needle_accepts_decimal_and_hex() {
        let mut search = RamSearch { needle: "255".into(), ..Default::default() };
        assert_eq!(search.parse_needle(), Some(255));
        search.needle = "0xFF".into();
        assert_eq!(search.parse_needle(), Some(255));
        search.needle = "  0x10  ".into();
        assert_eq!(search.parse_needle(), Some(16));
    }

    #[test]
    fn the_needle_rejects_nonsense_and_values_too_wide_for_the_width() {
        let mut search = RamSearch { needle: "abc".into(), ..Default::default() };
        assert_eq!(search.parse_needle(), None);

        search.needle = "300".into();
        search.width = SearchWidth::Byte;
        assert_eq!(search.parse_needle(), None, "300 does not fit in 8 bits");
        search.width = SearchWidth::Half;
        assert_eq!(search.parse_needle(), Some(300));
    }
}
