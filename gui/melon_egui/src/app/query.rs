//! Small getters and setters the menus and panes use, plus the OSD message
//! helpers (`post*`) every command reports through.

use super::*;

impl MelonEgui {
    /// While the Input dialog is waiting, turn the next key or pad press into
    /// a binding (Escape cancels). Runs before key sampling, so the press does
    /// not also reach the cart.
    pub(crate) fn poll_rebind(&mut self, ctx: &egui::Context) {
        let Some((input, device)) = self.listening else { return };

        let pressed = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Key { key, pressed: true, .. } => Some(*key),
                _ => None,
            })
        });
        if pressed == Some(egui::Key::Escape) {
            self.listening = None;
            return;
        }

        let bound = match device {
            crate::bindings::Device::Keyboard => {
                pressed.map(|key| self.bindings.bind_key(input, key)).is_some()
            }
            // Escape (above) also cancels a pad binding — the only way out when
            // the pad is unplugged.
            crate::bindings::Device::Pad => {
                self.pads.first_pressed().map(|b| self.bindings.bind_button(input, b)).is_some()
            }
        };
        if bound {
            self.listening = None;
            self.persist();
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.emu.is_some()
    }

    pub const fn is_paused(&self) -> bool {
        self.paused
    }

    /// The second console's frame count, or `None` without one. A number that
    /// keeps climbing shows the pair really runs concurrently.
    pub fn guest_frames(&self) -> Option<u32> {
        self.guest.as_ref().map(crate::guest::Guest::frame_count)
    }

    pub const fn has_guest(&self) -> bool {
        self.guest.is_some()
    }

    pub fn recent_roms(&self) -> &[PathBuf] {
        &self.recents
    }

    pub fn open_panes(&self) -> Vec<Pane> {
        self.panes.clone()
    }

    pub fn close_pane(&mut self, pane: Pane) {
        self.panes.retain(|open| *open != pane);
    }

    /// What melonDS shows next to "DS slot:".
    pub fn cart_label(&self) -> String {
        self.emu.as_ref().map_or_else(
            || self.i18n().s(K::None),
            |emu| {
                emu.rom_path.file_name().map_or_else(
                    || emu.rom_path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            },
        )
    }

    /// The game controllers the last repaint saw, for the Input pane.
    pub fn connected_pads(&self) -> &[String] {
        self.pads.connected()
    }

    /// `(lid closed, battery okay)` read from the core, or `None` without a
    /// cart. Read rather than mirrored, so a cart's own changes show.
    pub fn power_state(&mut self) -> Option<(bool, bool)> {
        let emu = self.emu.as_mut()?;
        Some((emu.nds.lid_closed(), emu.nds.battery_okay()))
    }

    pub fn set_lid_closed(&mut self, closed: bool) {
        if let Some(emu) = &mut self.emu {
            emu.nds.set_lid_closed(closed);
        }
    }

    pub fn set_battery_okay(&mut self, okay: bool) {
        if let Some(emu) = &mut self.emu {
            emu.nds.set_battery_okay(okay);
        }
    }

    /// The ROM info pane's rows: what each is called, and its value.
    pub fn cart_info(&self) -> Option<Vec<(K, String)>> {
        let emu = self.emu.as_ref()?;
        Some(vec![
            (K::InfoTitle, emu.info.title.clone()),
            (K::InfoGameCode, emu.info.gamecode.clone()),
            (K::InfoMakerCode, emu.info.maker.clone()),
            (K::InfoRomSize, format!("{:.1} MiB", emu.info.size as f64 / (1024.0 * 1024.0))),
            (K::InfoFile, emu.rom_path.display().to_string()),
        ])
    }

    pub fn state_slot_exists(&self, slot: u8) -> bool {
        self.emu.as_ref().is_some_and(|emu| emu.state_path(slot).exists())
    }

    pub const fn can_undo_state_load(&self) -> bool {
        self.undo_state.is_some()
    }

    /// What the Audio settings pane says about the device.
    pub fn audio_status(&self) -> Notice {
        match &self.audio {
            Ok(audio) => Notice::quiet(
                Severity::Success,
                self.i18n().f(K::AudioPlayingOn, &[&audio.description()]),
            ),
            Err(e) => Notice::quiet(Severity::Error, self.i18n().f(K::AudioNone, &[e])),
        }
    }

    /// Whether there is a device to configure at all.
    pub const fn has_audio(&self) -> bool {
        self.audio.is_ok()
    }

    pub fn volume(&self) -> f32 {
        self.audio.as_ref().map_or(1.0, |audio| audio.volume)
    }

    pub fn set_volume(&mut self, volume: f32) {
        if let Ok(audio) = &mut self.audio {
            audio.volume = volume;
        }
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, dark: bool) {
        self.dark_theme = dark;
        ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    }

    // -- the Date and time dialog -------------------------------------------

    /// Push the dialog's clock into the console.
    pub fn apply_clock(&mut self) {
        let clock = self.clock;
        let tr = self.translations.get(self.language);
        match &mut self.emu {
            Some(emu) => {
                emu.set_clock(clock);
                let when = format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                    clock.year, clock.month, clock.day, clock.hour, clock.minute, clock.second
                );
                self.clock_note = Notice::new(Severity::Success, tr.f(K::ClockSet, &[&when]));
            }
            None => self.clock_note = Notice::new(Severity::Warn, tr.s(K::NoCartLoaded)),
        }
        // Both consoles, always: two carts that disagree about the date behave
        // differently in any game that checks it, and on a link that is a
        // desync waiting to happen.
        if let Some(guest) = &self.guest {
            guest.send(crate::guest::Command::SetClock(clock));
        }
    }

    /// Post an OSD message from a pane, at whatever severity it earned.
    pub fn post_message(&mut self, severity: Severity, message: impl Into<String>) {
        self.osd = Some((Notice::new(severity, message), Instant::now()));
    }

    /// Show `dir` in the system file manager, creating it first.
    pub fn reveal(&mut self, dir: &Path) {
        if let Err(error) = std::fs::create_dir_all(dir) {
            self.post_error(self.i18n().f(K::CannotCreate, &[&dir.display(), &error]));
            return;
        }
        let command = if cfg!(windows) {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        match std::process::Command::new(command).arg(dir).spawn() {
            // `explorer` exits non-zero even on success, so a spawned child is
            // as much confirmation as there is to be had.
            Ok(_) => self.post_ok(self.i18n().f(K::Opened, &[&dir.display()])),
            Err(error) => self.post_error(self.i18n().f(K::CannotOpen, &[&dir.display(), &error])),
        }
    }

    /// Post a neutral OSD message: a state change worth mentioning.
    pub(crate) fn post(&mut self, message: impl Into<String>) {
        self.post_message(Severity::Info, message);
    }

    /// Post a failure. Red on screen, `error!` in the log.
    pub(crate) fn post_error(&mut self, message: impl Into<String>) {
        self.post_message(Severity::Error, message);
    }

    /// Post a caveat: it happened, but not as asked. Yellow, `warn!`.
    pub(crate) fn post_warn(&mut self, message: impl Into<String>) {
        self.post_message(Severity::Warn, message);
    }

    /// Post a success. Green on screen.
    pub(crate) fn post_ok(&mut self, message: impl Into<String>) {
        self.post_message(Severity::Success, message);
    }

    /// Open or close one of the auxiliary windows.
    pub fn toggle_pane(&mut self, pane: Pane) {
        if let Some(at) = self.panes.iter().position(|open| *open == pane) {
            self.panes.remove(at);
        } else {
            self.panes.push(pane);
        }
    }

    /// Whether the OpenGL renderers can be offered at all.
    pub const fn gl_available(&self) -> bool {
        self.gl_loaded && self.gl_screen.is_some()
    }

    /// The View options with `ScreenSizing::Auto` turned into whichever concrete
    /// sizing the console's current output calls for.
    pub(crate) fn resolved_view(&self) -> ViewOptions {
        ViewOptions {
            sizing: self.view.sizing.resolve(self.screens_live[0], self.screens_live[1]),
            ..self.view
        }
    }
}
