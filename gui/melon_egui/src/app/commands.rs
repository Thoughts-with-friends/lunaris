//! What a menu entry does, for each of the three kinds of window.
//!
//! A menu never acts directly: it returns an [`Action`], and one of these three
//! dispatchers turns it into work after the frame's UI has been drawn.
//!
//! | dispatcher                       | window                                  |
//! |----------------------------------|-----------------------------------------|
//! | [`MelonEgui::apply`]             | the main window (first console)         |
//! | [`MelonEgui::apply_to_guest`]    | the second console's window             |
//! | [`MelonEgui::apply_as_client`]   | a Remote Desktop client (no console)    |
//!
//! Each `match` is exhaustive on purpose: adding a menu entry is a compile
//! error until every kind of window has an answer for it. An entry that makes
//! no sense in a window says so on the OSD instead of silently acting on the
//! wrong console.

use super::*;

impl MelonEgui {
    /// Perform a menu action from the main window.
    pub(crate) fn apply(&mut self, action: Action, ctx: &egui::Context) {
        if !self.mode.emulates() {
            return self.apply_as_client(action, ctx);
        }
        match action {
            Action::OpenRom | Action::InsertCart => self.ask(
                DialogPurpose::OpenRom,
                crate::file::picker::Request::open("Open a Nintendo DS ROM")
                    .filter("Nintendo DS ROM", &["nds", "dsi", "srl"])
                    .directory(
                        self.recents.first().and_then(|rom| rom.parent().map(Path::to_path_buf)),
                    ),
            ),
            Action::EjectCart | Action::Stop => {
                self.unload_cart();
                self.post("cart ejected");
            }
            Action::OpenRecent(index) => {
                if let Some(rom) = self.recents.get(index).cloned() {
                    self.load(&rom);
                }
            }
            Action::ClearRecent => {
                self.clear_recent();
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
            Action::TogglePause => self.toggle_pause(),
            Action::Reset => {
                if let Some(emu) = &mut self.emu {
                    emu.nds.boot();
                    self.frames_run = 0;
                    self.post("reset");
                }
            }
            Action::FrameStep => {
                // Pause and owe one frame, as melonDS's frame step does.
                self.paused = true;
                self.step_pending = true;
            }
            Action::ScreenSize(scale) => self.resize_for_scale(ctx, scale),
            Action::LaunchInstance => self.launch_instance(),
            Action::HostLanGame => self.start_lan(true),
            Action::GuestLanGame => self.start_lan(false),
            Action::HostRemoteDesktop => self.start_remote(true),
            Action::JoinRemoteDesktop => self.start_remote(false),
            Action::StopRemoteDesktop => self.stop_remote(),
            Action::TogglePane(pane) => self.toggle_pane(pane),
        }
    }

    /// Perform a menu action from the second console's window.
    ///
    /// Console work is queued for the second console's thread
    /// ([`crate::guest::Command`]); window-only work is done here.
    pub(crate) fn apply_to_guest(&mut self, action: Action) {
        use crate::guest::Command;
        match action {
            Action::TogglePause => self.toggle_pause(),
            Action::Reset => self.command_guest(Command::Reset),
            Action::FrameStep => {
                self.paused = true;
                self.command_guest(Command::FrameStep);
            }
            Action::Stop | Action::EjectCart => {
                self.command_guest(Command::Stop);
                self.close_guest();
                self.post("second console stopped");
            }
            Action::SaveState(Some(slot)) => {
                self.command_guest(Command::SaveState(Some(slot), None));
            }
            Action::LoadState(Some(slot)) => {
                self.command_guest(Command::LoadState(Some(slot), None));
            }
            Action::SaveState(None) => self.ask_for_guest_file(
                DialogPurpose::GuestSaveState,
                crate::file::picker::Request::save("Save instance 2 state"),
                ("savestate", &["ml1"]),
                "states",
            ),
            Action::LoadState(None) => self.ask_for_guest_file(
                DialogPurpose::GuestLoadState,
                crate::file::picker::Request::open("Load instance 2 state"),
                ("savestate", &["ml1"]),
                "states",
            ),
            Action::UndoStateLoad => self.command_guest(Command::UndoStateLoad),
            Action::ImportSavefile => self.ask_for_guest_file(
                DialogPurpose::GuestImportSave,
                crate::file::picker::Request::open("Import a save into instance 2"),
                ("save file", &["sav", "dsv", "bin"]),
                "saves",
            ),
            Action::OpenDirectory => self.open_instance_directory(2),
            // Resizing is done against the second window's own context, in
            // `guest_view`; reaching here means that window has already gone.
            Action::ScreenSize(_) => {}
            Action::Quit => {
                self.close_guest();
                self.post("second console closed");
            }
            Action::ClearRecent => self.clear_recent(),
            Action::NewWindow => self.second_window = !self.second_window,
            Action::TogglePane(pane) => self.toggle_pane(pane),
            // These belong to the console that owns the window and the airwaves.
            Action::OpenRom
            | Action::InsertCart
            | Action::OpenRecent(_)
            | Action::LaunchInstance
            | Action::HostLanGame
            | Action::GuestLanGame
            | Action::HostRemoteDesktop
            | Action::JoinRemoteDesktop
            | Action::StopRemoteDesktop => {
                self.post_warn("that command belongs to the first console");
            }
        }
    }

    /// Perform a menu action on a Remote Desktop client, which has no console:
    /// the cart, saves and states all belong to the host.
    pub(crate) fn apply_as_client(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::StopRemoteDesktop | Action::Stop | Action::EjectCart => self.stop_remote(),
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::OpenDirectory => self.open_directory(),
            Action::ScreenSize(scale) => self.resize_for_scale(ctx, scale),
            Action::NewWindow => self.second_window = !self.second_window,
            Action::TogglePane(pane) => self.toggle_pane(pane),
            Action::ClearRecent => self.clear_recent(),
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
                self.post_warn(
                    "this window is a Remote Desktop client — the host owns the console",
                );
            }
        }
    }

    /// Pause or resume. Resuming starts a fresh pacing window, since time
    /// spent paused is not frames owed.
    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        self.last_tick = Instant::now();
        self.frame_debt = 0.0;
    }

    /// Forget the recent-ROM list, and save that.
    fn clear_recent(&mut self) {
        self.recents.clear();
        self.persist();
    }

    /// Resize the window so the screens are drawn at exactly `scale`.
    pub(crate) fn resize_for_scale(&self, ctx: &egui::Context, scale: f32) {
        let size = view::window_size_for_scale(scale, &self.view, CHROME_HEIGHT);
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    }

    /// Open a file dialog for the second console, starting in its own
    /// `instance2/<kind>` directory.
    fn ask_for_guest_file(
        &mut self,
        purpose: DialogPurpose,
        request: crate::file::picker::Request,
        (filter, extensions): (&str, &[&str]),
        kind: &str,
    ) {
        let dir = crate::file::settings::instance_data_dir(2, kind);
        self.ask(purpose, request.filter(filter, extensions).directory(Some(dir)));
    }
}
