//! What a menu entry does, for each of the three kinds of window.
//!
//! Three dispatchers, each an exhaustive `match`: the first console
//! ([`MelonEgui::apply`]), the second ([`MelonEgui::apply_to_guest`]), and a
//! Remote Desktop client ([`MelonEgui::apply_as_client`]). Exhaustive on
//! purpose — adding an entry should be a compile error until somebody has
//! decided what each window does with it.

use super::*;

impl MelonEgui {
    /// Perform a menu action for the second console rather than the first.
    ///
    /// The second console's window draws the same menu bar, and until this
    /// existed every entry in it acted on the *first* console — which is why
    /// only the entries that happen to be pure UI appeared to work there. What
    /// cannot be done for the second console (opening a different cart, LAN,
    /// launching a third) says so instead of silently doing it to the first.
    pub(crate) fn apply_to_guest(&mut self, action: Action) {
        use crate::guest::Command;
        match action {
            Action::TogglePause => {
                self.paused = !self.paused;
                self.last_tick = Instant::now();
                self.frame_debt = 0.0;
            }
            Action::Reset => self.command_guest(Command::Reset),
            Action::FrameStep => {
                self.paused = true;
                self.command_guest(Command::FrameStep);
            }
            Action::Stop | Action::EjectCart => {
                self.command_guest(Command::Stop);
                self.guest = None;
                self.guest_textures = None;
                self.post("second console stopped");
            }
            Action::SaveState(Some(slot)) => {
                self.command_guest(Command::SaveState(Some(slot), None));
            }
            Action::LoadState(Some(slot)) => {
                self.command_guest(Command::LoadState(Some(slot), None));
            }
            Action::SaveState(None) => self.ask(
                DialogPurpose::GuestSaveState,
                crate::fs::Request::save("Save instance 2 state")
                    .filter("savestate", &["ml1"])
                    .directory(Some(crate::config::instance_data_dir(2, "states"))),
            ),
            Action::LoadState(None) => self.ask(
                DialogPurpose::GuestLoadState,
                crate::fs::Request::open("Load instance 2 state")
                    .filter("savestate", &["ml1"])
                    .directory(Some(crate::config::instance_data_dir(2, "states"))),
            ),
            Action::UndoStateLoad => self.command_guest(Command::UndoStateLoad),
            Action::ImportSavefile => self.ask(
                DialogPurpose::GuestImportSave,
                crate::fs::Request::open("Import a save into instance 2")
                    .filter("save file", &["sav", "dsv", "bin"])
                    .directory(Some(crate::config::instance_data_dir(2, "saves"))),
            ),
            Action::OpenDirectory => self.open_instance_directory(2),
            // Handled against the guest viewport's own context, in
            // `guest_view`; reaching here means the window had already gone.
            Action::ScreenSize(_) => {}
            Action::Quit => {
                self.guest = None;
                self.guest_textures = None;
                self.post("second console closed");
            }
            // These belong to the console that owns the airwaves and the
            // window, so they are refused rather than misapplied.
            Action::OpenRom
            | Action::InsertCart
            | Action::OpenRecent(_)
            | Action::LaunchInstance
            | Action::HostLanGame
            | Action::GuestLanGame
            | Action::HostRemoteDesktop
            | Action::JoinRemoteDesktop
            | Action::StopRemoteDesktop => {
                self.post("that command belongs to the first console");
            }
            // Purely the window's own business, and already handled where the
            // guest window collected it.
            other => self.apply_ui_only(other),
        }
    }

    /// The actions that change how a window looks rather than what a console
    /// does, which are the same for either console.
    pub(crate) fn apply_ui_only(&mut self, action: Action) {
        match action {
            Action::ClearRecent => {
                self.recents.clear();
                self.persist();
            }
            Action::NewWindow => self.second_window = !self.second_window,
            Action::TogglePane(pane) => self.toggle_pane(pane),
            // `ScreenSize` resizes the window it was clicked in, which the
            // guest window handles itself; everything else is already covered.
            _ => {}
        }
    }

    /// Perform a menu action on a window that emulates nothing.
    ///
    /// A Remote Desktop client has no cart, no save and no savestate — they all
    /// belong to the host, which is the point of the mode. Rather than let
    /// those entries appear to work and silently do nothing, everything that
    /// needs a console says where it actually lives.
    ///
    /// Exhaustive on purpose: adding a menu entry should be a compile error
    /// here until somebody has decided what a client does with it.
    pub(crate) fn apply_as_client(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::StopRemoteDesktop | Action::Stop | Action::EjectCart => self.stop_remote(),
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::OpenDirectory => self.open_directory(),
            Action::ScreenSize(scale) => {
                let size = view::window_size_for_scale(scale, &self.view, CHROME_HEIGHT);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
            Action::NewWindow => {
                self.second_window = !self.second_window;
            }
            Action::TogglePane(pane) => self.toggle_pane(pane),
            Action::ClearRecent => {
                self.recents.clear();
                self.persist();
            }
            // Everything below drives a console. There is not one here.
            Action::OpenRom
            | Action::OpenRecent(_)
            | Action::InsertCart
            | Action::ImportSavefile
            | Action::SaveState(_)
            | Action::LoadState(_)
            | Action::UndoStateLoad
            | Action::TogglePause
            | Action::Reset
            | Action::FrameStep
            | Action::LaunchInstance
            | Action::HostLanGame
            | Action::GuestLanGame
            | Action::HostRemoteDesktop
            | Action::JoinRemoteDesktop => {
                self.post("this window is a Remote Desktop client — the host owns the console");
            }
        }
    }

    pub(crate) fn apply(&mut self, action: Action, ctx: &egui::Context) {
        // A window that emulates nothing cannot run an emulator's commands.
        if !self.mode.emulates() {
            return self.apply_as_client(action, ctx);
        }
        match action {
            Action::OpenRom | Action::InsertCart => self.ask(
                DialogPurpose::OpenRom,
                crate::fs::Request::open("Open a Nintendo DS ROM")
                    .filter("Nintendo DS ROM", &["nds", "dsi", "srl"])
                    .directory(
                        self.recents.first().and_then(|rom| rom.parent().map(Path::to_path_buf)),
                    ),
            ),
            Action::EjectCart | Action::Stop => {
                self.emu = None;
                self.drop_link();
                self.textures = None;
                self.undo_state = None;
                self.post("cart ejected");
            }
            Action::OpenRecent(index) => {
                if let Some(rom) = self.recents.get(index).cloned() {
                    self.load(&rom);
                }
            }
            Action::ClearRecent => {
                self.recents.clear();
                self.persist();
                self.post("recent list cleared");
            }
            Action::OpenDirectory => self.open_directory(),
            Action::NewWindow => {
                self.second_window = !self.second_window;
                let opened = self.second_window;
                self.post(if opened { "second window opened" } else { "second window closed" });
            }
            Action::ImportSavefile => self.import_savefile(),
            Action::SaveState(slot) => self.save_state(slot),
            Action::LoadState(slot) => self.load_state(slot),
            Action::UndoStateLoad => self.undo_state_load(),
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::TogglePause => {
                self.paused = !self.paused;
                // Resuming starts a fresh pacing window: time spent paused is
                // not frames owed.
                self.last_tick = Instant::now();
                self.frame_debt = 0.0;
            }
            Action::Reset => {
                if let Some(emu) = &mut self.emu {
                    emu.nds.boot();
                    self.frames_run = 0;
                    self.post("reset");
                }
            }
            Action::FrameStep => {
                // melonDS's frame step pauses and advances by one, so holding
                // the command walks the console forward frame by frame.
                self.paused = true;
                self.step_pending = true;
            }
            Action::ScreenSize(scale) => {
                let size = view::window_size_for_scale(scale, &self.view, CHROME_HEIGHT);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
            Action::LaunchInstance => self.launch_instance(),
            Action::HostLanGame => self.start_lan(true),
            Action::GuestLanGame => self.start_lan(false),
            Action::HostRemoteDesktop => self.start_remote(true),
            Action::JoinRemoteDesktop => self.start_remote(false),
            Action::StopRemoteDesktop => self.stop_remote(),
            Action::TogglePane(pane) => {
                if let Some(at) = self.panes.iter().position(|open| *open == pane) {
                    self.panes.remove(at);
                } else {
                    self.panes.push(pane);
                }
            }
        }
    }
}
