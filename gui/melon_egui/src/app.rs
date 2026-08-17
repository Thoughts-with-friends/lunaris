//! The egui application: pacing the core, blitting its framebuffers, and
//! feeding it keys and touch.
//!
//! The window's shape follows melonDS's — a menu bar over the screens, with
//! messages drawn over the picture as an OSD rather than in a status bar. The
//! menu itself lives in [`crate::menu`].

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use egui::{Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, pos2};
use melonds::{SCREEN_HEIGHT, SCREEN_WIDTH, keys};

use crate::{
    config::Settings,
    emu::Emu,
    menu::{self, Action},
    panes,
    view::{self, Rotation, ViewOptions},
};

/// The DS video frame rate: `33_513_982 / 560_190` Hz. Slightly under the 60 Hz
/// a display usually runs at, so pacing has to come from a clock rather than
/// from one frame per repaint.
const FRAME_RATE: f64 = 59.826_1;

/// How many emulated frames a single repaint may run to catch up. A window that
/// was dragged or occluded can leave an arbitrarily large debt; running all of
/// it would make the picture lurch, so the surplus is dropped instead.
const MAX_CATCH_UP: u32 = 4;

/// Emulated frames per repaint while a `--shot` capture is pending, or while the
/// framerate limiter is off. Large enough to be much faster than real time,
/// small enough that the window still pumps its event loop in between.
const UNLIMITED_BURST: u32 = 64;

/// How often pending backup memory is written to disk.
const SAVE_FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// How long an OSD message stays up.
const OSD_LIFETIME: Duration = Duration::from_secs(3);

/// Room the menu bar takes, in points. Used to size the window so that the
/// screens land on an exact scale.
const CHROME_HEIGHT: f32 = 26.0;

/// Numbered savestate slots, as melonDS offers.
pub const STATE_SLOTS: u8 = 8;

pub use crate::{
    config::RECENT_LIMIT,
    panes::{Pane, RamSearch},
};

/// The size the window opens at: both screens at 2x, which is legible without
/// filling a modern display.
pub fn default_window_size() -> [f32; 2] {
    view::window_size_for_scale(2.0, &ViewOptions::default(), CHROME_HEIGHT).into()
}

/// The floor, at 1x. Below this the screens would have to be scaled down, which
/// for pixel art is worse than a small window.
pub fn min_window_size() -> [f32; 2] {
    view::window_size_for_scale(1.0, &ViewOptions::default(), CHROME_HEIGHT).into()
}

/// Keyboard bindings, matching melonDS's defaults.
pub const BINDINGS: &[(egui::Key, u32, &str)] = &[
    (egui::Key::X, keys::A, "A"),
    (egui::Key::Z, keys::B, "B"),
    (egui::Key::S, keys::X, "X"),
    (egui::Key::A, keys::Y, "Y"),
    (egui::Key::Q, keys::L, "L"),
    (egui::Key::W, keys::R, "R"),
    (egui::Key::Enter, keys::START, "Start"),
    (egui::Key::Backspace, keys::SELECT, "Select"),
    (egui::Key::ArrowUp, keys::UP, "Up"),
    (egui::Key::ArrowDown, keys::DOWN, "Down"),
    (egui::Key::ArrowLeft, keys::LEFT, "Left"),
    (egui::Key::ArrowRight, keys::RIGHT, "Right"),
];

pub struct MelonEgui {
    emu: Option<Emu>,
    /// Uploaded once per emulated frame; `[top, bottom]`.
    textures: Option<[TextureHandle; 2]>,
    paused: bool,
    /// One frame is owed even though the core is paused — the Frame step
    /// command, which is the only way to advance while stopped.
    step_pending: bool,
    pub view: ViewOptions,
    /// Whether to pace the core against wall-clock time. Off, it runs as fast
    /// as it can, matching melonDS's "Limit framerate".
    pub limit_framerate: bool,
    /// White noise on the microphone, the only mic input this build has.
    pub mic_static: bool,
    pub pause_when_unfocused: bool,
    pub confirm_on_quit: bool,
    pub dark_theme: bool,
    pub ui_scale: f32,
    /// Directory overrides; `None` means "beside the ROM".
    pub save_dir: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    /// The Date and time dialog's working copy, applied on its button.
    pub clock: crate::emu::Clock,
    pub clock_note: String,
    pub ram_search: RamSearch,
    /// Recently opened ROMs, newest first.
    recents: Vec<PathBuf>,
    /// Which auxiliary windows are open.
    panes: Vec<Pane>,
    /// Whether the second view of this console is open.
    second_window: bool,
    /// Whether each screen had anything on it last frame, which is what
    /// `ScreenSizing::Auto` decides on.
    screens_live: [bool; 2],
    /// Fractional emulated frames owed, carried across repaints so a 60 Hz
    /// display does not slowly outrun the DS's 59.83 Hz.
    frame_debt: f64,
    last_tick: Instant,
    last_save_flush: Instant,
    /// Where the bottom screen was drawn last repaint. Touch is sampled before
    /// this repaint's layout runs, so it uses the previous rectangle — one
    /// repaint of latency, invisible at these sizes. `None` when the bottom
    /// screen is not shown, which makes it untouchable, as it should be.
    bottom_screen: Option<Rect>,
    /// Emulated frames run and the wall-clock window they took, for the
    /// throughput readout.
    fps_frames: u32,
    fps_since: Instant,
    fps: f64,
    /// The newest OSD message and when it was posted.
    osd: Option<(String, Instant)>,
    /// The state the console was in before the last `Load state`, so that it can
    /// be taken back.
    undo_state: Option<Vec<u8>>,
    /// Emulated frames run since the cart booted, for [`Self::service_shot`].
    frames_run: u64,
    /// `--shot`: capture the window once this many frames have run, write it
    /// there, and quit. `None` in normal use.
    shot: Option<(u64, PathBuf)>,
    /// Whether the capture has already been asked for, so it is asked for once.
    shot_requested: bool,
}

impl MelonEgui {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        rom: Option<PathBuf>,
        shot: Option<(u64, PathBuf)>,
    ) -> Self {
        let settings = Settings::load();
        let now = Instant::now();
        let mut app = Self {
            emu: None,
            textures: None,
            paused: false,
            step_pending: false,
            view: settings.view,
            limit_framerate: settings.limit_framerate,
            mic_static: false,
            pause_when_unfocused: false,
            confirm_on_quit: false,
            dark_theme: settings.dark_theme,
            ui_scale: if settings.ui_scale > 0.0 { settings.ui_scale } else { 1.0 },
            save_dir: settings.save_dir,
            state_dir: settings.state_dir,
            clock: crate::emu::utc_clock(),
            clock_note: String::new(),
            ram_search: RamSearch::default(),
            recents: settings.recents,
            panes: Vec::new(),
            second_window: false,
            screens_live: [true, true],
            frame_debt: 0.0,
            last_tick: now,
            last_save_flush: now,
            bottom_screen: None,
            fps_frames: 0,
            fps_since: now,
            fps: 0.0,
            osd: None,
            undo_state: None,
            frames_run: 0,
            shot,
            shot_requested: false,
        };
        app.set_theme(&cc.egui_ctx, app.dark_theme);
        if settings.ui_scale > 0.0 {
            cc.egui_ctx.set_zoom_factor(settings.ui_scale);
        }
        match rom {
            Some(rom) => app.load(&rom),
            None => app.post("no cart loaded — File ▸ Open ROM..."),
        }
        app
    }

    /// Record `rom` at the top of the recent list and save.
    fn push_recent(&mut self, rom: &Path) {
        let mut settings = self.settings();
        settings.push_recent(rom);
        self.recents = settings.recents.clone();
        settings.save();
    }

    /// Collect everything worth remembering and write it out.
    fn persist(&self) {
        self.settings().save();
    }

    /// Everything worth remembering, gathered up.
    fn settings(&self) -> Settings {
        Settings {
            recents: self.recents.clone(),
            view: self.view,
            limit_framerate: self.limit_framerate,
            audio_sync: false,
            state_dir: self.state_dir.clone(),
            save_dir: self.save_dir.clone(),
            ui_scale: self.ui_scale,
            dark_theme: self.dark_theme,
        }
    }

    // -- state the menu and the panes ask about -----------------------------

    pub fn is_loaded(&self) -> bool {
        self.emu.is_some()
    }

    pub const fn is_paused(&self) -> bool {
        self.paused
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

    /// The ROM info pane's rows, or `None` with no cart loaded.
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

    /// Why there is no sound, for the Audio settings pane.
    pub fn audio_status(&self) -> &'static str {
        "No audio output: this front end has no audio device backend yet. \
         The core does produce sound (the FFI exposes it), so this is a gap here, \
         not a limit of the bindings."
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
    }

    // -- the RAM search -----------------------------------------------------

    /// Read the value at `addr` at the search's current width.
    pub fn ram_read(&mut self, addr: u32) -> u32 {
        let Some(emu) = &mut self.emu else { return 0 };
        match self.ram_search.width {
            crate::panes::SearchWidth::Byte => u32::from(emu.nds.read8(addr)),
            crate::panes::SearchWidth::Half => u32::from(emu.nds.read16(addr)),
            crate::panes::SearchWidth::Word => emu.nds.read32(addr),
        }
    }

    /// Scan the whole of main RAM for the value, replacing any previous results.
    pub fn ram_first_scan(&mut self) {
        let Some(needle) = self.ram_search.parse_needle() else { return };
        let width = self.ram_search.width;
        let Some(emu) = &mut self.emu else { return };

        // Main RAM starts at 0200_0000h on both CPUs (GBATEK, "Memory Maps").
        const MAIN_RAM_BASE: u32 = 0x0200_0000;
        let len = emu.nds.main_ram().len();
        let mut hits = Vec::new();
        let stride = width.size();
        for offset in (0..len.saturating_sub(stride - 1)).step_by(stride) {
            let addr = MAIN_RAM_BASE + offset as u32;
            let value = match width {
                crate::panes::SearchWidth::Byte => u32::from(emu.nds.read8(addr)),
                crate::panes::SearchWidth::Half => u32::from(emu.nds.read16(addr)),
                crate::panes::SearchWidth::Word => emu.nds.read32(addr),
            };
            if value == needle {
                hits.push(addr);
            }
        }
        let found = hits.len();
        self.ram_search.hits = hits;
        self.post(format!("RAM search: {found} addresses hold {needle}"));
    }

    /// Keep only the addresses that still hold the value.
    pub fn ram_narrow(&mut self) {
        let Some(needle) = self.ram_search.parse_needle() else { return };
        let width = self.ram_search.width;
        let Some(emu) = &mut self.emu else { return };

        let before = self.ram_search.hits.len();
        self.ram_search.hits.retain(|&addr| {
            let value = match width {
                crate::panes::SearchWidth::Byte => u32::from(emu.nds.read8(addr)),
                crate::panes::SearchWidth::Half => u32::from(emu.nds.read16(addr)),
                crate::panes::SearchWidth::Word => emu.nds.read32(addr),
            };
            value == needle
        });
        let after = self.ram_search.hits.len();
        self.post(format!("RAM search: narrowed {before} to {after}"));
    }

    // -- commands -----------------------------------------------------------

    /// Post an OSD message. Also where every command reports its outcome, so
    /// that failures are visible without a console.
    fn post(&mut self, message: impl Into<String>) {
        let message = message.into();
        eprintln!("melon_egui: {message}");
        self.osd = Some((message, Instant::now()));
    }

    /// Boot `rom`, replacing whatever was running.
    fn load(&mut self, rom: &Path) {
        // Dropped first so the outgoing cart's save is flushed before the
        // incoming one can be handed the same file.
        self.emu = None;
        self.undo_state = None;
        self.textures = None;
        self.frames_run = 0;
        match Emu::boot_with(rom, self.save_dir.as_ref(), self.state_dir.as_ref()) {
            Ok(emu) => {
                self.emu = Some(emu);
                self.push_recent(rom);
                self.paused = false;
                self.frame_debt = 0.0;
                self.last_tick = Instant::now();
                self.post(format!("loaded {}", rom.display()));
            }
            Err(e) => self.post(format!("failed to load {}: {e}", rom.display())),
        }
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::OpenRom | Action::InsertCart => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Nintendo DS ROM", &["nds", "dsi", "srl"])
                    .pick_file()
                {
                    self.load(&path);
                }
            }
            Action::EjectCart | Action::Stop => {
                self.emu = None;
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
            Action::TogglePane(pane) => {
                if let Some(at) = self.panes.iter().position(|open| *open == pane) {
                    self.panes.remove(at);
                } else {
                    self.panes.push(pane);
                }
            }
        }
    }

    /// Show this front end's directory in the system file manager.
    fn open_directory(&mut self) {
        let dir = crate::config::config_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.post(format!("cannot create {}: {e}", dir.display()));
            return;
        }
        let command = if cfg!(windows) {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        match std::process::Command::new(command).arg(&dir).spawn() {
            // `explorer` exits non-zero even on success, so a spawned child is
            // as much confirmation as there is to be had.
            Ok(_) => self.post(format!("opened {}", dir.display())),
            Err(e) => self.post(format!("cannot open {}: {e}", dir.display())),
        }
    }

    fn import_savefile(&mut self) {
        let Some(path) =
            rfd::FileDialog::new().add_filter("save file", &["sav", "dsv", "bin"]).pick_file()
        else {
            return;
        };
        let outcome = std::fs::read(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))
            .and_then(|data| {
                self.emu
                    .as_mut()
                    .ok_or_else(|| "no cart loaded".to_owned())
                    .and_then(|emu| emu.import_save(&data))
            });
        match outcome {
            // Importing restarts the console, so nothing from before survives.
            Ok(()) => {
                self.undo_state = None;
                self.frames_run = 0;
                self.post(format!("imported {} — console restarted", path.display()));
            }
            Err(e) => self.post(format!("import failed: {e}")),
        }
    }

    fn save_state(&mut self, slot: Option<u8>) {
        let Some(emu) = &mut self.emu else { return };
        let path = match slot {
            Some(slot) => emu.state_path(slot),
            None => {
                let Some(path) =
                    rfd::FileDialog::new().add_filter("savestate", &["ml1"]).save_file()
                else {
                    return;
                };
                path
            }
        };
        let mut buf = Vec::new();
        let outcome = emu.nds.save_state(&mut buf).map_err(|e| e.to_string()).and_then(|()| {
            std::fs::write(&path, &buf).map_err(|e| format!("cannot write {}: {e}", path.display()))
        });
        match outcome {
            Ok(()) => {
                let mib = buf.len() as f64 / (1024.0 * 1024.0);
                self.post(format!("state saved to {} ({mib:.1} MiB)", path.display()));
            }
            Err(e) => self.post(format!("save state failed: {e}")),
        }
    }

    fn load_state(&mut self, slot: Option<u8>) {
        let Some(emu) = &mut self.emu else { return };
        let path = match slot {
            Some(slot) => emu.state_path(slot),
            None => {
                let Some(path) =
                    rfd::FileDialog::new().add_filter("savestate", &["ml1"]).pick_file()
                else {
                    return;
                };
                path
            }
        };

        // Snapshot first: a load with nothing to go back to is a load that
        // cannot be undone, and melonDS offers exactly that undo.
        let mut before = Vec::new();
        let snapshot = emu.nds.save_state(&mut before).is_ok();

        let outcome = std::fs::read(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))
            .and_then(|buf| emu.nds.load_state(&buf).map_err(|e| e.to_string()));
        match outcome {
            Ok(()) => {
                self.undo_state = snapshot.then_some(before);
                self.post(format!("state loaded from {}", path.display()));
            }
            Err(e) => self.post(format!("load state failed: {e}")),
        }
    }

    fn undo_state_load(&mut self) {
        let Some(emu) = &mut self.emu else { return };
        let Some(before) = self.undo_state.take() else {
            return;
        };
        match emu.nds.load_state(&before) {
            Ok(()) => self.post("state load undone"),
            Err(e) => self.post(format!("undo failed: {e}")),
        }
    }

    // -- the emulation loop -------------------------------------------------

    /// Run however many emulated frames wall-clock time has earned, then upload
    /// the resulting picture.
    fn advance(&mut self, ctx: &egui::Context) {
        let elapsed = self.last_tick.elapsed();
        self.last_tick = Instant::now();

        if self.emu.is_none() {
            return;
        }

        // melonDS's "pause when unfocused": the console keeps its state, it just
        // stops advancing while the window is in the background.
        let unfocused = self.pause_when_unfocused && !ctx.input(|i| i.focused);
        let due = if self.paused || unfocused {
            // Debt accrued while paused is not owed: resuming should not
            // fast-forward through it.
            self.frame_debt = 0.0;
            u32::from(std::mem::take(&mut self.step_pending))
        } else if self.shot.is_some() || !self.limit_framerate {
            UNLIMITED_BURST
        } else {
            self.frame_debt += elapsed.as_secs_f64() * FRAME_RATE;
            let due = (self.frame_debt as u32).min(MAX_CATCH_UP);
            self.frame_debt -= f64::from(due);
            // Whatever is still owed after the cap is dropped rather than
            // carried, so a stall cannot turn into a burst later.
            if self.frame_debt > f64::from(MAX_CATCH_UP) {
                self.frame_debt = 0.0;
            }
            due
        };

        // Sampled before the core is borrowed, since both readings come out of
        // `self` and the emulator borrow would conflict with them.
        let keys = ctx.input(|i| {
            BINDINGS
                .iter()
                .filter(|(key, ..)| i.key_down(*key))
                .fold(0, |mask, (_, bit, _)| mask | bit)
        });
        let touch = self.sample_touch(ctx);

        let mut ran = 0;
        let mut stopped = false;
        let mic_static = self.mic_static;
        if let Some(emu) = &mut self.emu {
            emu.nds.set_keys(keys);
            emu.set_mic_static(mic_static);
            match touch {
                Some((x, y)) => emu.nds.touch(x, y),
                None => emu.nds.release_screen(),
            }
            for _ in 0..due {
                if emu.nds.run_frame() == 0 {
                    stopped = true;
                    break;
                }
                ran += 1;
            }
        }
        self.fps_frames += ran;
        self.frames_run += u64::from(ran);
        if stopped {
            self.paused = true;
            self.post("core stopped");
        }

        if ran > 0 {
            self.upload(ctx);
        }

        let window = self.fps_since.elapsed();
        if window >= Duration::from_secs(1) {
            self.fps = f64::from(self.fps_frames) / window.as_secs_f64();
            self.fps_frames = 0;
            self.fps_since = Instant::now();
        }

        if self.last_save_flush.elapsed() >= SAVE_FLUSH_INTERVAL {
            self.last_save_flush = Instant::now();
            if let Some(emu) = &self.emu {
                emu.flush_save();
            }
        }
    }

    /// The pointer's position on the bottom screen in touchscreen coordinates,
    /// or `None` when the stylus is not down on it.
    fn sample_touch(&self, ctx: &egui::Context) -> Option<(u16, u16)> {
        let rect = self.bottom_screen?;
        let pos =
            ctx.input(|i| i.pointer.primary_down().then(|| i.pointer.interact_pos()).flatten())?;
        touch_coords(rect, pos, self.view.rotation)
    }

    /// Copy both framebuffers into egui textures.
    fn upload(&mut self, ctx: &egui::Context) {
        let filter =
            if self.view.filtering { TextureOptions::LINEAR } else { TextureOptions::NEAREST };
        let Some(emu) = &mut self.emu else {
            return;
        };
        let Some((top, bottom)) = emu.nds.framebuffers() else {
            return;
        };
        // What `ScreenSizing::Auto` decides on: a screen showing nothing but
        // black is one the console is not really using.
        let lit = |fb: &[u32]| fb.iter().any(|&px| px & 0x00FF_FFFF != 0);
        let live = [lit(top), lit(bottom)];
        let images = [to_image(top), to_image(bottom)];
        self.screens_live = live;

        match &mut self.textures {
            // The options go in on every upload, so toggling `Screen filtering`
            // takes effect on the next frame without rebuilding the textures.
            Some(textures) => {
                for (texture, image) in textures.iter_mut().zip(images) {
                    texture.set(image, filter);
                }
            }
            None => {
                let [top, bottom] = images;
                self.textures = Some([
                    ctx.load_texture("ds-top", top, filter),
                    ctx.load_texture("ds-bottom", bottom, filter),
                ]);
            }
        }
    }

    // -- drawing ------------------------------------------------------------

    /// Lay the screens out in `area` and paint them, recording where the bottom
    /// one landed so the next repaint can map touch onto it.
    fn screens(&mut self, ui: &mut egui::Ui, area: Rect) {
        let placed = view::layout(area, &self.resolved_view());
        self.bottom_screen = placed.bottom;

        let Some(textures) = &self.textures else {
            return;
        };
        let painter = ui.painter();
        for (rect, texture) in [(placed.top, &textures[0]), (placed.bottom, &textures[1])] {
            if let Some(rect) = rect {
                paint_screen(painter, texture.id(), rect, self.view.rotation);
            }
        }
    }

    /// The View options with `ScreenSizing::Auto` turned into whichever concrete
    /// sizing the console's current output calls for.
    fn resolved_view(&self) -> ViewOptions {
        ViewOptions {
            sizing: self.view.sizing.resolve(self.screens_live[0], self.screens_live[1]),
            ..self.view
        }
    }

    /// The OSD: melonDS draws its messages and its frame rate over the picture
    /// rather than in a status bar, so this front end does too.
    fn osd(&mut self, ui: &mut egui::Ui, area: Rect) {
        if !self.view.show_osd {
            return;
        }
        // Only the newest message, and only while it is fresh.
        let mut lines = Vec::new();
        if let Some((message, at)) = &self.osd {
            if at.elapsed() < OSD_LIFETIME {
                lines.push(message.clone());
            } else {
                self.osd = None;
            }
        }
        if self.is_loaded() {
            let paused = if self.paused { "  [paused]" } else { "" };
            lines.insert(0, format!("{:.1} FPS{paused}", self.fps));
        }

        let painter = ui.painter();
        let mut at = area.left_top() + egui::vec2(6.0, 4.0);
        for line in lines {
            // Drawn twice, offset, so the text stays readable over both a light
            // and a dark picture — the cheap equivalent of an outline.
            for (offset, color) in [(1.0, Color32::BLACK), (0.0, Color32::WHITE)] {
                painter.text(
                    at + egui::vec2(offset, offset),
                    egui::Align2::LEFT_TOP,
                    &line,
                    egui::FontId::monospace(13.0),
                    color,
                );
            }
            at.y += 16.0;
        }
    }

    /// A second window showing the same console, as melonDS's "Open new window"
    /// does. It shares the textures, so it costs a blit and no emulation.
    fn second_view(&mut self, ctx: &egui::Context) {
        if !self.second_window {
            return;
        }
        let Some(textures) = self.textures.clone() else {
            return;
        };
        let view = self.resolved_view();
        let id = egui::ViewportId::from_hash_of("melon_egui-second-view");
        let builder = egui::ViewportBuilder::default()
            .with_title("melon_egui - second view")
            .with_inner_size(default_window_size())
            // Same reason main.rs needs it: winit's drag-and-drop support
            // initialises COM as an STA, which conflicts with an MTA already
            // established on this process.
            .with_drag_and_drop(false);

        let mut closed = false;
        ctx.show_viewport_immediate(id, builder, |ctx, _class| {
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(
                ctx,
                |ui| {
                    let area = ui.max_rect();
                    let placed = view::layout(area, &view);
                    let painter = ui.painter();
                    for (rect, texture) in
                        [(placed.top, &textures[0]), (placed.bottom, &textures[1])]
                    {
                        if let Some(rect) = rect {
                            paint_screen(painter, texture.id(), rect, view.rotation);
                        }
                    }
                },
            );
            if ctx.input(|i| i.viewport().close_requested()) {
                closed = true;
            }
        });
        if closed {
            self.second_window = false;
        }
    }

    /// Drive a pending `--shot`: ask for the capture once the cart has run far
    /// enough, then write whatever egui hands back and quit.
    ///
    /// The image arrives on a later repaint as an [`egui::Event::Screenshot`],
    /// because the frame has to reach the GPU before it can be read back.
    fn service_shot(&mut self, ctx: &egui::Context) {
        let Some((at, path)) = &self.shot else {
            return;
        };

        if let Some(image) = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(std::sync::Arc::clone(image)),
                _ => None,
            })
        }) {
            let rgba: Vec<u8> = image.pixels.iter().flat_map(Color32::to_array).collect();
            let [w, h] = image.size;
            let result = image::save_buffer(
                path,
                &rgba,
                w as u32,
                h as u32,
                image::ExtendedColorType::Rgba8,
            );
            match result {
                Ok(()) => println!("shot: wrote {} ({w}x{h})", path.display()),
                Err(e) => eprintln!("shot: failed to write {}: {e}", path.display()),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if !self.shot_requested && self.frames_run >= *at {
            self.shot_requested = true;
            println!("shot: {} frames run, requesting capture", self.frames_run);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }
}

impl eframe::App for MelonEgui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.advance(ctx);

        let mut action = None;
        egui::TopBottomPanel::top("menu").show(ctx, |ui| action = menu::bar(self, ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(
            ctx,
            |ui| {
                let area = ui.max_rect();
                self.screens(ui, area);
                self.osd(ui, area);
            },
        );
        panes::show(self, ctx);
        self.second_view(ctx);
        if let Some(action) = action {
            self.apply(action, ctx);
        }

        self.service_shot(ctx);

        // The core is paced off wall-clock time, so the window has to keep
        // repainting rather than wait for input. Paused, there is nothing to
        // redraw until something happens.
        if self.emu.is_some() && (!self.paused || self.step_pending) {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(emu) = &self.emu {
            emu.flush_save();
        }
        self.persist();
    }
}

/// Paint one screen into `rect`, rotated.
///
/// Rotation is done by permuting the texture coordinates rather than by
/// transforming the destination: `rect` is already the shape the rotated screen
/// occupies, so the only question is which corner of the picture goes where.
fn paint_screen(painter: &egui::Painter, texture: egui::TextureId, rect: Rect, rotation: Rotation) {
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    let mut mesh = egui::Mesh::with_texture(texture);
    for (pos, uv) in corners.into_iter().zip(uv_corners(rotation)) {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// Which corner of the texture each destination corner samples, in the order
/// [`paint_screen`] walks them: clockwise from the top-left.
///
/// Turning the picture `n` quarter turns clockwise means each destination corner
/// shows what sat `n` corners anticlockwise of it in the source.
fn uv_corners(rotation: Rotation) -> [Pos2; 4] {
    /// The whole texture's corners, clockwise from the top-left.
    const UV: [Pos2; 4] = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
    std::array::from_fn(|i| UV[(i + 4 - rotation.steps()) % 4])
}

/// Where `pos` lands on a bottom screen drawn at `rect` under `rotation`, in
/// touchscreen coordinates, or `None` when it is off the panel.
///
/// Split out from [`MelonEgui::sample_touch`] so the arithmetic — the part that
/// changes with every layout option — is testable without a window.
fn touch_coords(rect: Rect, pos: Pos2, rotation: Rotation) -> Option<(u16, u16)> {
    if !rect.contains(pos) {
        return None;
    }
    // Position within the drawn panel, as a fraction of it.
    let u = (pos.x - rect.left()) / rect.width();
    let v = (pos.y - rect.top()) / rect.height();
    // Undo the rotation: this is the inverse of the permutation
    // `paint_screen` applies to the texture coordinates.
    let (sx, sy) = match rotation {
        Rotation::None => (u, v),
        Rotation::Cw90 => (v, 1.0 - u),
        Rotation::Cw180 => (1.0 - u, 1.0 - v),
        Rotation::Cw270 => (1.0 - v, u),
    };
    // The touchscreen has no sub-pixel resolution, and coordinates past the
    // panel are not something the hardware can report, so the scaled position
    // is floored and clamped. `rect.contains` is inclusive of the far edge,
    // which is exactly the case the clamp catches.
    Some((
        ((sx * SCREEN_WIDTH as f32) as u16).min(SCREEN_WIDTH as u16 - 1),
        ((sy * SCREEN_HEIGHT as f32) as u16).min(SCREEN_HEIGHT as u16 - 1),
    ))
}

/// A melonDS framebuffer as an egui image.
///
/// The core hands over one `u32` per pixel as `0xAARRGGBB` — byte order BGRA in
/// memory, which is what melonDS calls the format (`GPU_Soft.cpp`, "convert to
/// 32-bit BGRA"). Alpha is whatever the compositor left there, so it is
/// discarded and the pixel forced opaque.
fn to_image(fb: &[u32]) -> ColorImage {
    let pixels = fb
        .iter()
        .map(|&px| Color32::from_rgb((px >> 16) as u8, (px >> 8) as u8, px as u8))
        .collect();
    ColorImage {
        size: [SCREEN_WIDTH, SCREEN_HEIGHT],
        pixels,
        source_size: egui::vec2(SCREEN_WIDTH as f32, SCREEN_HEIGHT as f32),
    }
}

#[cfg(test)]
mod tests {
    use egui::{Color32, Pos2, Rect, pos2, vec2};
    use melonds::{SCREEN_HEIGHT, SCREEN_WIDTH};

    use super::{Rotation, to_image, touch_coords};

    /// A bottom screen drawn at 3x, offset so that a bug that forgets to
    /// subtract the rectangle's origin cannot pass by coincidence.
    fn screen_rect() -> Rect {
        Rect::from_min_size(
            pos2(40.0, 300.0),
            vec2(SCREEN_WIDTH as f32 * 3.0, SCREEN_HEIGHT as f32 * 3.0),
        )
    }

    #[test]
    fn touch_maps_the_panel_corners_to_the_panel_corners() {
        let rect = screen_rect();
        assert_eq!(touch_coords(rect, rect.min, Rotation::None), Some((0, 0)));
        assert_eq!(
            touch_coords(rect, rect.max, Rotation::None),
            Some((SCREEN_WIDTH as u16 - 1, SCREEN_HEIGHT as u16 - 1)),
            "the far corner is inclusive, so it must clamp inside the panel",
        );
    }

    #[test]
    fn touch_maps_the_panel_centre_to_the_panel_centre() {
        let rect = screen_rect();
        assert_eq!(
            touch_coords(rect, rect.center(), Rotation::None),
            Some((SCREEN_WIDTH as u16 / 2, SCREEN_HEIGHT as u16 / 2)),
        );
    }

    #[test]
    fn touch_scales_by_the_drawn_size_not_by_pixels() {
        // A quarter of the way across a 3x panel is a quarter of the way across
        // the touchscreen, whatever the scale.
        let rect = screen_rect();
        let pos = rect.min + vec2(rect.width() / 4.0, rect.height() / 4.0);
        assert_eq!(
            touch_coords(rect, pos, Rotation::None),
            Some((SCREEN_WIDTH as u16 / 4, SCREEN_HEIGHT as u16 / 4)),
        );
    }

    #[test]
    fn touch_outside_the_panel_is_not_a_touch() {
        let rect = screen_rect();
        for outside in [
            pos2(rect.left() - 1.0, rect.center().y),
            pos2(rect.center().x, rect.top() - 1.0),
            pos2(rect.right() + 1.0, rect.center().y),
            pos2(rect.center().x, rect.bottom() + 1.0),
            Pos2::ZERO,
        ] {
            assert_eq!(touch_coords(rect, outside, Rotation::None), None, "at {outside:?}");
        }
    }

    /// Rotating the picture has to rotate the touch map with it, or the stylus
    /// lands somewhere other than where the player is pointing.
    #[test]
    fn touch_follows_the_rotation() {
        let rect = screen_rect();
        // Turned a quarter clockwise, the panel's top-left corner shows the
        // picture's bottom-left, so touching there is touching (0, max).
        assert_eq!(
            touch_coords(rect, rect.left_top(), Rotation::Cw90),
            Some((0, SCREEN_HEIGHT as u16 - 1)),
        );
        assert_eq!(
            touch_coords(rect, rect.left_top(), Rotation::Cw180),
            Some((SCREEN_WIDTH as u16 - 1, SCREEN_HEIGHT as u16 - 1)),
        );
        assert_eq!(
            touch_coords(rect, rect.left_top(), Rotation::Cw270),
            Some((SCREEN_WIDTH as u16 - 1, 0)),
        );
        // The centre is the centre whichever way up it is.
        for rotation in Rotation::ALL {
            assert_eq!(
                touch_coords(rect, rect.center(), rotation),
                Some((SCREEN_WIDTH as u16 / 2, SCREEN_HEIGHT as u16 / 2)),
                "{rotation:?}",
            );
        }
    }

    /// The property that actually matters about rotation: whatever the painter
    /// puts on screen, the touch map has to be its exact inverse, or the stylus
    /// lands somewhere other than where the player is pointing.
    ///
    /// Checked by taking each corner of the drawn panel, reading off which
    /// corner of the *picture* the painter shows there, and confirming the touch
    /// map reports that same corner.
    #[test]
    fn the_touch_map_inverts_what_the_painter_draws() {
        use super::uv_corners;
        let rect = screen_rect();
        let panel = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];

        for rotation in Rotation::ALL {
            for (corner, uv) in panel.into_iter().zip(uv_corners(rotation)) {
                // The texture corner the painter samples there, in touchscreen
                // coordinates, clamped the way the touch map clamps.
                let expected = (
                    ((uv.x * SCREEN_WIDTH as f32) as u16).min(SCREEN_WIDTH as u16 - 1),
                    ((uv.y * SCREEN_HEIGHT as f32) as u16).min(SCREEN_HEIGHT as u16 - 1),
                );
                assert_eq!(
                    touch_coords(rect, corner, rotation),
                    Some(expected),
                    "{rotation:?} at {corner:?}",
                );
            }
        }
    }

    /// The core hands over `0xAARRGGBB`; a swapped red and blue channel is the
    /// classic way to get a picture that is present but wrong, and it survives
    /// every "is it black?" check.
    #[test]
    fn framebuffer_words_keep_their_channel_order() {
        let mut fb = vec![0u32; SCREEN_WIDTH * SCREEN_HEIGHT];
        fb[0] = 0xFF_12_34_56;
        fb[1] = 0x00_FF_00_00; // pure red, transparent: alpha must be ignored
        let image = to_image(&fb);
        assert_eq!(image.size, [SCREEN_WIDTH, SCREEN_HEIGHT]);
        assert_eq!(image.pixels[0], Color32::from_rgb(0x12, 0x34, 0x56));
        assert_eq!(image.pixels[1], Color32::from_rgb(0xFF, 0, 0));
        assert_eq!(image.pixels[2], Color32::BLACK);
    }
}
