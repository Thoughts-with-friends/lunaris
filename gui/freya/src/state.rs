//! App-wide state handed to every component through freya's context.
//!
//! ```text
//!  Frontend (frontend.rs, Arc)        config.json, emulator thread, actions
//!     |  refresh() after every action / twice a second
//!     v
//!  Ctx { config, menus, status, dialog, error, look, ... }  (freya States)
//!     |  use_consume::<Ctx>()
//!     v
//!  titlebar / menubar / screens / status bar / dialogs
//! ```
//!
//! The 60 Hz frames are not in here: only the screens component holds them,
//! so a new frame does not re-render the menus or an open dialog.
use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc};

use ds_common::{
    frontend::{GuiConfig, file_name, lock, rom_paths},
    menu::{Menu, MenuState, Window, menu_bar},
};
use freya::prelude::*;
use serde::{Deserialize, Serialize};

use crate::frontend::Frontend;

/// Why LAN play is not offered here.
pub const NO_MULTIPLAYER: &str =
    "LAN play is only available in the egui GUI for now (Launch new instance works)";

/// What the status bar shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub running: bool,
    pub paused: bool,
    pub fps: f32,
    /// File name of the running game.
    pub rom: Option<String>,
    pub rom_path: Option<PathBuf>,
    pub instance: u32,
}

/// Look of this GUI (`freya.json` in the instance folder; the shared
/// `config.json` is left to [`GuiConfig`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Look {
    /// Opacity of the window body, 0.3-1.0 (the DS screens stay opaque).
    pub opacity: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self { opacity: 0.82 }
    }
}

/// Everything the components share.
#[derive(Clone)]
pub struct Ctx {
    pub front: Arc<Frontend>,
    pub config: State<GuiConfig>,
    pub menus: State<Vec<Menu>>,
    pub status: State<Status>,
    /// The open dialog.
    pub dialog: State<Option<Window>>,
    /// The Appearance popover of the titlebar.
    pub appearance: State<bool>,
    /// Last error, shown until dismissed.
    pub error: State<Option<String>>,
    pub look: State<Look>,
    /// Names of the connected gamepads (Input dialog).
    pub gamepads: State<Vec<String>>,
    /// DS buttons held on the keyboard (`emu_thread::key` bits).
    pub keyboard: Rc<Cell<u32>>,
}

impl Ctx {
    /// Creates the states (call once, in the root component).
    pub fn new(front: Arc<Frontend>) -> Self {
        let look_path = front.instance.root().join("freya.json");
        let look = std::fs::read_to_string(look_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let config = lock(&front.config).clone();
        Self {
            config: State::create(config),
            menus: State::create(Vec::new()),
            status: State::create(Status::default()),
            dialog: State::create(None),
            appearance: State::create(false),
            error: State::create(None),
            look: State::create(look),
            gamepads: State::create(Vec::new()),
            keyboard: Rc::new(Cell::new(0)),
            front,
        }
    }

    /// Reads config, menus and status back from the [`Frontend`].
    pub fn refresh(&self) {
        let mut config = self.config;
        let next = lock(&self.front.config).clone();
        if *config.peek() != next {
            config.set(next);
        }
        let mut menus = self.menus;
        let next = self.build_menus();
        if *menus.peek() != next {
            menus.set(next);
        }
        self.refresh_status();
    }

    /// Only the status (fps, pause, game).
    pub fn refresh_status(&self) {
        let rom_path = lock(&self.front.rom).clone();
        let next = Status {
            running: self.front.shared.is_running() && lock(&self.front.emu).is_some(),
            paused: self.front.shared.is_paused(),
            fps: self.front.shared.fps(),
            rom: rom_path.as_deref().map(file_name),
            rom_path,
            instance: self.front.instance.number(),
        };
        let mut status = self.status;
        if *status.peek() != next {
            status.set(next);
        }
    }

    /// The shared menu tree (same as egui / Tauri).
    fn build_menus(&self) -> Vec<Menu> {
        let config = lock(&self.front.config).clone();
        let loaded = lock(&self.front.rom).as_deref().map(file_name);
        let folder_roms = rom_paths(&config.rom_dir);
        menu_bar(&MenuState {
            loaded: loaded.as_deref(),
            paused: self.front.shared.is_paused(),
            view: &config.view,
            limit_framerate: config.limit_framerate,
            recent: &config.recent_roms,
            folder_roms: &folder_roms,
            multiplayer: Err(NO_MULTIPLAYER),
        })
    }

    /// Changes the GUI config (volume, ROM folder, ...) and saves it.
    pub fn update_config(&self, change: impl FnOnce(&mut GuiConfig)) {
        let mut config = lock(&self.front.config).clone();
        change(&mut config);
        self.front.set_config(config);
        self.refresh();
    }

    /// Changes the look and saves `freya.json`.
    pub fn update_look(&self, change: impl FnOnce(&mut Look)) {
        let mut look = self.look;
        let mut next = look.peek().clone();
        change(&mut next);
        if let Ok(text) = serde_json::to_string_pretty(&next) {
            let _ = std::fs::write(self.front.instance.root().join("freya.json"), text);
        }
        look.set(next);
    }

    pub fn show_error(&self, error: impl Into<String>) {
        let mut state = self.error;
        state.set(Some(error.into()));
    }
}
