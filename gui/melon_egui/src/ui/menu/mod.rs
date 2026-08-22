//! The menu bar, built to melonDS's shape.
//!
//! Structure, wording and offered values are transcribed from melonDS's
//! `frontend/qt_sdl/Window.cpp` (the `menubar->addMenu(...)` blocks), so that
//! someone who knows melonDS finds the same commands in the same places.
//!
//! Every entry that *can* be backed is live. The rest are **shown but disabled**
//! rather than omitted, each carrying the specific reason it cannot work — see
//! [`Unavailable`]. The shape then matches melonDS's, and what is missing is
//! visible in the UI instead of merely absent.

use egui::Ui;

use crate::{app::MelonEgui, i18n::I18nKey as K, ui::panes::Pane};

mod config;
mod file;
mod help;
mod system;
mod view;

use config::config_menu;
use file::file_menu;
use help::help_menu;
use system::system_menu;
use view::view_menu;

/// Why a menu entry is disabled.
#[derive(Clone, Copy)]
enum Unavailable {
    /// The melonDS core can do it, but `melonds-rs`'s FFI (`shim.h`) exposes no
    /// entry point for it, so no front end built on these bindings can reach it.
    Bindings,
}

impl Unavailable {
    /// The key whose text explains this, so the reason is translated along with
    /// everything else rather than being the one English string left on a
    /// Japanese menu.
    const fn key(self) -> K {
        match self {
            Self::Bindings => K::UnavailableBindings,
        }
    }
}

/// What a menu entry asks for. Returned rather than acted on so that the menu
/// closure does not need `&mut` access to the app while egui holds it.
pub enum Action {
    OpenRom,
    /// One of the remembered ROMs, by index into the recent list.
    OpenRecent(usize),
    ClearRecent,
    InsertCart,
    EjectCart,
    ImportSavefile,
    /// `Some(slot)` for one of the numbered slots, `None` for "File...".
    SaveState(Option<u8>),
    LoadState(Option<u8>),
    UndoStateLoad,
    /// Reveal the directory this front end keeps its files in.
    OpenDirectory,
    Quit,
    TogglePause,
    Reset,
    Stop,
    FrameStep,
    /// Resize the window so the screens land on exactly this scale.
    ScreenSize(f32),
    /// A second window showing the same console.
    NewWindow,
    /// Open (or close) a second console on the shared airwaves.
    LaunchInstance,
    /// Accept one remote LAN console on the configured UDP port.
    HostLanGame,
    /// Connect this console to the configured LAN host.
    GuestLanGame,
    /// Run both consoles here and stream the second one out. See
    /// [`crate::remote`].
    HostRemoteDesktop,
    /// Become a screen for a console running elsewhere.
    JoinRemoteDesktop,
    /// End whichever Remote Desktop session is running.
    StopRemoteDesktop,
    /// Show or hide one of the auxiliary windows.
    TogglePane(Pane),
}

/// Draw the bar, returning whichever entry was clicked.
pub fn bar(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    egui::MenuBar::new().ui(ui, |ui| {
        action = file_menu(app, ui)
            .or_else(|| system_menu(app, ui))
            .or_else(|| view_menu(app, ui))
            .or_else(|| config_menu(app, ui))
            .or_else(|| help_menu(app, ui));
    });
    action
}

/// An entry that is present for shape but cannot be used, with the reason on
/// hover.
///
/// Takes the app so both the label and the reason come out of the translation
/// map; a menu with one English tooltip on it reads as an oversight.
fn unavailable(app: &MelonEgui, ui: &mut Ui, label: K, why: Unavailable) {
    let (label, reason) = (app.i18n().s(label), app.i18n().s(why.key()));
    ui.add_enabled(false, egui::Button::new(label)).on_disabled_hover_text(reason);
}

/// An entry that runs `action` and closes the menu.
fn entry(ui: &mut Ui, enabled: bool, label: &str, action: Action) -> Option<Action> {
    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
        ui.close();
        return Some(action);
    }
    None
}

/// A translated entry, which is what nearly every entry here is.
///
/// The label is copied out of the map before the widget is built: `app.i18n`
/// borrows `app`, and several call sites need `&mut app` in the same
/// expression.
fn item(app: &MelonEgui, ui: &mut Ui, enabled: bool, label: K, action: Action) -> Option<Action> {
    entry(ui, enabled, &app.i18n().s(label), action)
}
