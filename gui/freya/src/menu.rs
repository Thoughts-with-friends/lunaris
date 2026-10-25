//! The menu tree of this GUI: the tree shared with the egui and Tauri GUIs
//! (`ds_common::menu`), with the entries `ds_common` keeps disabled but
//! this GUI can back (savestates, Export savefile) turned into [`Act`]s.
//!
//! ```text
//!  ds_common::menu::menu_bar --from_ds--> Vec<Menu> --patch_file--> MenuBar
//!     File > Save state / Load state  1-8, to / from file  (cores with states)
//!     File > Export savefile...       after Import savefile...
//! ```
use std::path::PathBuf;

use ds_common::menu::{self as ds, Action};

use crate::debug::DebugTab;

/// What a menu entry asks for: a shared [`Action`], or one of this GUI's.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Ds(Action),
    /// File > Save state > N (Shift+F1-F8).
    SaveState(u8),
    /// File > Load state > N (F1-F8).
    LoadState(u8),
    SaveStateFile,
    LoadStateFile,
    /// Copy the game's save to a picked file.
    ExportSave,
    /// Back to before the last state load.
    UndoStateLoad,
    /// Debug > a view: open it in the side panel, or close it.
    Debug(DebugTab),
}

impl From<Action> for Act {
    fn from(action: Action) -> Self {
        Self::Ds(action)
    }
}

/// [`ds::Item`] with an [`Act`].
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Entry {
        label: String,
        action: Option<Act>,
        enabled: bool,
        checked: Option<bool>,
        radio: bool,
        reason: Option<String>,
        shortcut: Option<String>,
        hint: Option<String>,
    },
    Submenu {
        label: String,
        items: Vec<Self>,
    },
    Separator,
    Label {
        text: String,
    },
}

impl Item {
    fn entry(label: &str, action: Act, enabled: bool, reason: &str) -> Self {
        let shortcut = match action {
            Act::SaveState(n) => Some(format!("Shift+F{n}")),
            Act::LoadState(n) => Some(format!("F{n}")),
            _ => None,
        };
        Self::Entry {
            label: label.to_owned(),
            action: Some(action),
            enabled,
            checked: None,
            radio: false,
            reason: (!enabled).then(|| reason.to_owned()),
            shortcut,
            hint: None,
        }
    }

    fn from_ds(item: ds::Item) -> Self {
        match item {
            ds::Item::Entry { label, action, enabled, checked, radio, reason, shortcut, hint } => {
                Self::Entry {
                    label,
                    action: action.map(Act::Ds),
                    enabled,
                    checked,
                    radio,
                    reason,
                    shortcut,
                    hint,
                }
            }
            ds::Item::Submenu { label, items, .. } => {
                Self::Submenu { label, items: items.into_iter().map(Self::from_ds).collect() }
            }
            ds::Item::Separator => Self::Separator,
            ds::Item::Label { text } => Self::Label { text },
        }
    }

    const fn label(&self) -> Option<&String> {
        match self {
            Self::Entry { label, .. } | Self::Submenu { label, .. } => Some(label),
            _ => None,
        }
    }
}

/// A top-level menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub label: String,
    pub items: Vec<Item>,
}

/// What the entries added here depend on.
pub struct Extra {
    /// A game is running.
    pub loaded: bool,
    /// Savestate files of slots 1-8 (`None`: this core has no savestates).
    pub slots: Option<[Option<PathBuf>; 8]>,
    /// The open Debug view.
    pub debug: Option<DebugTab>,
    /// A state load can be undone.
    pub can_undo: bool,
}

/// The shared tree with this GUI's entries.
pub fn menus(shared: Vec<ds::Menu>, extra: &Extra) -> Vec<Menu> {
    let mut menus: Vec<Menu> = shared
        .into_iter()
        .map(|menu| {
            let mut items: Vec<Item> = menu.items.into_iter().map(Item::from_ds).collect();
            if menu.label == "File" {
                patch_file(&mut items, extra);
            }
            Menu { label: menu.label, items }
        })
        .collect();
    let help = menus.iter().position(|m| m.label == "Help").unwrap_or(menus.len());
    menus.insert(help, debug_menu(extra.debug));
    menus
}

/// Debug: the egui GUI's inspector windows, as views of the side panel.
fn debug_menu(open: Option<DebugTab>) -> Menu {
    let items = DebugTab::ALL
        .into_iter()
        .map(|tab| Item::Entry {
            label: tab.label().to_owned(),
            action: Some(Act::Debug(tab)),
            enabled: tab.available(),
            checked: Some(open == Some(tab)),
            radio: false,
            reason: (!tab.available())
                .then(|| "VRAM views are only in the Lunaris core".to_owned()),
            shortcut: None,
            hint: None,
        })
        .collect();
    Menu { label: "Debug".to_owned(), items }
}

fn patch_file(items: &mut Vec<Item>, extra: &Extra) {
    use ds::reason::NOT_LOADED;

    if let Some(slots) = &extra.slots {
        for item in items.iter_mut() {
            let load = match item.label().map(String::as_str) {
                Some("Save state") => false,
                Some("Load state") => true,
                _ => continue,
            };
            let mut entries: Vec<Item> = (1..=8_u8)
                .zip(slots)
                .map(|(n, file)| {
                    let (action, enabled, reason) = if load {
                        let enabled = extra.loaded && file.is_some();
                        let why =
                            if extra.loaded { "Nothing saved in this slot" } else { NOT_LOADED };
                        (Act::LoadState(n), enabled, why)
                    } else {
                        (Act::SaveState(n), extra.loaded, NOT_LOADED)
                    };
                    let mut entry = Item::entry(&n.to_string(), action, enabled, reason);
                    if let (Item::Entry { hint, .. }, Some(file)) = (&mut entry, file) {
                        *hint = Some(file.display().to_string());
                    }
                    entry
                })
                .collect();
            entries.push(Item::Separator);
            entries.push(if load {
                Item::entry("Load from file...", Act::LoadStateFile, extra.loaded, NOT_LOADED)
            } else {
                Item::entry("Save to file...", Act::SaveStateFile, extra.loaded, NOT_LOADED)
            });
            *item =
                Item::Submenu { label: item.label().cloned().unwrap_or_default(), items: entries };
        }
        if let Some(undo) =
            items.iter_mut().find(|it| it.label().is_some_and(|l| l == "Undo state load"))
        {
            let why = if extra.loaded { "No state load to undo" } else { NOT_LOADED };
            let enabled = extra.loaded && extra.can_undo;
            *undo = Item::entry("Undo state load", Act::UndoStateLoad, enabled, why);
        }
    }
    if let Some(i) =
        items.iter().position(|it| it.label().is_some_and(|l| l == "Import savefile..."))
    {
        items.insert(
            i + 1,
            Item::entry("Export savefile...", Act::ExportSave, extra.loaded, NOT_LOADED),
        );
    }
}

#[cfg(test)]
mod tests {
    use ds_common::{frontend::GuiConfig, menu::MenuState};

    use super::*;

    fn file_menu(extra: &Extra) -> Vec<Item> {
        let config = GuiConfig::default();
        let shared = ds::menu_bar(&MenuState {
            loaded: extra.loaded.then_some("game.nds"),
            paused: false,
            view: &config.view,
            limit_framerate: true,
            recent: &[],
            folder_roms: &[],
            multiplayer: Err("no"),
        });
        menus(shared, extra).remove(0).items
    }

    fn find<'a>(items: &'a [Item], label: &str) -> &'a Item {
        items.iter().find(|it| it.label().is_some_and(|l| l == label)).expect(label)
    }

    #[test]
    fn savestate_slots_follow_the_files() {
        let mut slots: [Option<PathBuf>; 8] = Default::default();
        slots[2] = Some(PathBuf::from("state_3.bin"));
        let items =
            file_menu(&Extra { loaded: true, slots: Some(slots), debug: None, can_undo: false });

        let Item::Submenu { items: save, .. } = find(&items, "Save state") else { panic!() };
        assert!(matches!(
            &save[0],
            Item::Entry { action: Some(Act::SaveState(1)), enabled: true, .. }
        ));
        let Item::Submenu { items: load, .. } = find(&items, "Load state") else { panic!() };
        assert!(matches!(&load[0], Item::Entry { enabled: false, .. }));
        assert!(matches!(
            &load[2],
            Item::Entry { action: Some(Act::LoadState(3)), enabled: true, .. }
        ));
        assert!(matches!(
            find(&items, "Export savefile..."),
            Item::Entry { action: Some(Act::ExportSave), enabled: true, .. }
        ));
    }

    #[test]
    fn a_core_without_states_keeps_them_disabled() {
        let items = file_menu(&Extra { loaded: true, slots: None, debug: None, can_undo: false });
        let Item::Submenu { items: save, .. } = find(&items, "Save state") else { panic!() };
        assert!(matches!(&save[0], Item::Entry { enabled: false, action: None, .. }));
    }
}
