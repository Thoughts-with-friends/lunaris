//! The menu bar, laid out like melonDS's own (`frontend/qt_sdl/Window.cpp`).
//!
//! # How a click becomes work
//!
//! ```text
//!  bar(app, ui)
//!   ├ file_menu / system_menu / view_menu / config_menu / help_menu
//!   │    each entry: Picked::item(...) → records the clicked Action
//!   └ returns Option<Action>
//!  update() then calls app.apply(action)        (app/commands.rs)
//! ```
//!
//! Some entries only toggle a value directly (checkboxes such as "Enable
//! cheats", the View menu's radio buttons); everything else is an [`Action`].
//!
//! Entries melonDS has but the bindings cannot back are **shown disabled**
//! with the reason on hover ([`Unavailable`]), so the menu keeps melonDS's
//! shape and what is missing is visible.

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

/// The entry clicked while the menus were drawn, if any.
///
/// Menus run inside egui closures that cannot hand `&mut app` back, so a
/// click is recorded here and returned once drawing is done. The first click
/// wins; once one is recorded, the remaining entries of that frame are not
/// drawn (the menu is closing anyway).
#[derive(Default)]
struct Picked(Option<Action>);

impl Picked {
    /// An entry labelled `label` that records `action` and closes the menu.
    fn entry(&mut self, ui: &mut Ui, enabled: bool, label: &str, action: Action) {
        if self.0.is_none() && ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            ui.close();
            self.0 = Some(action);
        }
    }

    /// A translated entry, which is what nearly every entry is.
    fn item(&mut self, app: &MelonEgui, ui: &mut Ui, enabled: bool, label: K, action: Action) {
        // Copied out first: `app.i18n` borrows `app`.
        let label = app.i18n().s(label);
        self.entry(ui, enabled, &label, action);
    }

    /// A translated entry that opens or closes one of the auxiliary windows.
    fn pane(&mut self, app: &MelonEgui, ui: &mut Ui, enabled: bool, label: K, pane: Pane) {
        self.item(app, ui, enabled, label, Action::TogglePane(pane));
    }
}
