//! Small questions the UI asks, and small things it sets.
//!
//! Nothing here is more than a few lines: these are the accessors the menu
//! and the panes need so they do not have to reach into the fields.

use super::*;

impl MelonEgui {
    pub fn is_loaded(&self) -> bool {
        self.emu.is_some()
    }

    pub const fn is_paused(&self) -> bool {
        self.paused
    }

    /// Whether a second console is running.
    /// The second console's frame count, or `None` when there is no second
    /// console. Its thread publishes this each frame, so a number that keeps
    /// climbing is the visible proof that the pair really is running
    /// concurrently rather than taking turns.
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
            || "(none)".to_owned(),
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

    /// The console's power state as `(lid closed, battery okay)`, or `None`
    /// with no cart running.
    ///
    /// Read from the core rather than mirrored here, so a cart that opens the
    /// lid itself shows up in the dialog.
    pub fn power_state(&mut self) -> Option<(bool, bool)> {
        let emu = self.emu.as_mut()?;
        Some((emu.lid_closed(), emu.battery_okay()))
    }

    pub fn set_lid_closed(&mut self, closed: bool) {
        if let Some(emu) = &mut self.emu {
            emu.set_lid_closed(closed);
        }
    }

    pub fn set_battery_okay(&mut self, okay: bool) {
        if let Some(emu) = &mut self.emu {
            emu.set_battery_okay(okay);
        }
    }

    pub fn cart_info(&self) -> Option<Vec<(&'static str, String)>> {
        let emu = self.emu.as_ref()?;
        Some(vec![
            ("Title", emu.info.title.clone()),
            ("Game code", emu.info.gamecode.clone()),
            ("Maker code", emu.info.maker.clone()),
            ("ROM size", format!("{:.1} MiB", emu.info.size as f64 / (1024.0 * 1024.0))),
            ("File", emu.rom_path.display().to_string()),
        ])
    }

    pub fn state_slot_exists(&self, slot: u8) -> bool {
        self.emu.as_ref().is_some_and(|emu| emu.state_path(slot).exists())
    }

    pub const fn can_undo_state_load(&self) -> bool {
        self.undo_state.is_some()
    }

    /// What the Audio settings pane says about the device.
    pub fn audio_status(&self) -> String {
        match &self.audio {
            Ok(audio) => format!("Playing on {}", audio.description()),
            Err(e) => format!("No audio output: {e}"),
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
        match &mut self.emu {
            Some(emu) => {
                emu.set_clock(clock);
                self.clock_note = format!(
                    "set to {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                    clock.year, clock.month, clock.day, clock.hour, clock.minute, clock.second
                );
            }
            None => self.clock_note = "no cart loaded".to_owned(),
        }
        // Both consoles, always: two carts that disagree about the date behave
        // differently in any game that checks it, and on a link that is a
        // desync waiting to happen.
        if let Some(guest) = &self.guest {
            guest.send(crate::guest::Command::SetClock(clock));
        }
    }

    // -- the RAM search -----------------------------------------------------

    /// Post an OSD message from a pane.
    pub fn post_message(&mut self, message: impl Into<String>) {
        self.post(message);
    }

    /// Show `dir` in the system file manager, creating it first.
    pub fn reveal(&mut self, dir: &Path) {
        if let Err(error) = std::fs::create_dir_all(dir) {
            self.post(format!("cannot create {}: {error}", dir.display()));
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
            Ok(_) => self.post(format!("opened {}", dir.display())),
            Err(error) => self.post(format!("cannot open {}: {error}", dir.display())),
        }
    }

    /// Recompute the throughput readout from the frames counted since the last
    /// window closed.
    ///
    /// Split out so a Remote Desktop client — which counts *received* frames
    /// rather than emulated ones — reports its rate the same way, and the
    /// number in the corner means "frames a second on this screen" in both
    /// modes.
    pub(crate) fn report_fps(&mut self) {
        let elapsed = self.fps_since.elapsed();
        if elapsed >= Duration::from_millis(500) {
            self.fps = f64::from(self.fps_frames) / elapsed.as_secs_f64();
            self.fps_frames = 0;
            self.fps_since = Instant::now();
        }
    }

    /// Post an OSD message. Also where every command reports its outcome, so
    /// that failures are visible without a console.
    pub(crate) fn post(&mut self, message: impl Into<String>) {
        let message = message.into();
        eprintln!("melon_egui: {message}");
        self.osd = Some((message, Instant::now()));
    }

    /// Open or close one of the auxiliary windows.
    pub fn toggle_pane(&mut self, pane: Pane) {
        if let Some(at) = self.panes.iter().position(|open| *open == pane) {
            self.panes.remove(at);
        } else {
            self.panes.push(pane);
        }
    }

    /// Push the Video settings' two core-side knobs, when they have changed.
    ///
    /// The screen mask is taken from the *explicit* sizings only. Under
    /// `ScreenSizing::Auto` it would feed back on itself: hiding a screen stops
    /// it being composed, its framebuffer goes stale, and the staleness is then
    /// read as the screen being idle.
    /// Whether the OpenGL renderer can be offered at all.
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
