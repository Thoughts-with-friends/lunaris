//! The tool and settings windows behind the menu's dialog entries.
//!
//! Each open window is a [`Pane`] in `app.panes`; [`show`] draws every open
//! one each repaint and drops those the user closed. Unlike melonDS's modal Qt
//! dialogs these never block emulation, and the open set is saved between runs.
//!
//! | file             | panes                                                |
//! |------------------|------------------------------------------------------|
//! | `settings.rs`    | Emu, Preferences, Video, Audio, Input                |
//! | `console.rs`     | Power, Date and time, ROM info, Crash report         |
//! | `cheat_codes.rs` | Cheat codes (list + editor, drag to reorder)         |
//! | `wireless.rs`    | Wireless status (+ `remote.rs`: Remote Desktop part) |
//! | `interface.rs`   | Interface (language, theme, scale), About            |
//! | `paths.rs`       | Path settings                                        |
//! | `ram_search.rs`  | RAM search                                           |

use egui::Context;

use crate::{
    app::MelonEgui,
    file::settings as config,
    i18n::I18nKey as K,
    mp::Kind,
    ui::{notice::Severity, view::AspectRatio},
    upscale,
    video::Renderer,
};

mod cheat_codes;
mod console;
mod interface;
mod paths;
mod ram_search;
mod remote;
mod settings;
mod wireless;

pub use cheat_codes::CheatEditor;
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
    /// The window title's key.
    pub const fn title(self) -> K {
        match self {
            Self::RomInfo => K::RomInfo,
            Self::Power => K::PowerManagement,
            Self::Cheats => K::CheatCodesTitle,
            Self::Crash => K::CrashTitle,
            Self::RamSearch => K::RamSearch,
            Self::DateTime => K::DateAndTime,
            Self::Input => K::InputAndHotkeys,
            Self::EmuSettings => K::EmuSettings,
            Self::Preferences => K::PreferencesTitle,
            Self::VideoSettings => K::VideoSettings,
            Self::AudioSettings => K::AudioSettings,
            Self::Wireless => K::WirelessStatus,
            Self::Interface => K::InterfaceSettings,
            Self::Paths => K::PathSettings,
            Self::About => K::AboutTitle,
        }
    }

    /// The window's egui identity: the pane itself, never its title, so a
    /// language switch keeps each window where it was and the same size.
    pub fn id(self) -> egui::Id {
        egui::Id::new(("melon_egui-pane", self))
    }
}

/// Draw every open pane, closing any whose window was dismissed.
pub fn show(app: &mut MelonEgui, ctx: &Context) {
    for pane in app.open_panes() {
        let mut open = true;
        let title = app.i18n().s(pane.title());
        egui::Window::new(title)
            .id(pane.id())
            .open(&mut open)
            .resizable(matches!(
                pane,
                Pane::RamSearch | Pane::Wireless | Pane::Cheats | Pane::Crash | Pane::Input
            ))
            // The three that are wider by nature: the wireless dialog is a
            // table of counters, Input is three columns of bindings, and the
            // cheat editor is a list beside a detail panel.
            .default_width(match pane {
                Pane::Cheats => 700.0,
                Pane::Wireless | Pane::Input => 460.0,
                _ => 260.0,
            })
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
        Pane::About => about(app, ui),
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
