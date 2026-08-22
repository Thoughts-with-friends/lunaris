//! Starting up, and everything remembered between runs.

use super::*;

impl MelonEgui {
    /// `renderer` is `--renderer`'s override of the saved Video settings, for
    /// this run only — see `crate::take_renderer`.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        rom: Option<PathBuf>,
        shot: Option<(u64, PathBuf)>,
        renderer: Option<(crate::video::Renderer, u32)>,
        launch_second: bool,
    ) -> Self {
        let settings = Settings::load();
        let instance2_settings = Settings::load_for(2);
        let now = Instant::now();
        let mut app = Self {
            emu: None,
            // Every language, once. A translation file that is present but
            // broken must not stop the emulator starting: the built-in text
            // stands in and the reason is said out loud.
            translations: crate::i18n::Translations::load(),
            language: settings.language,
            textures: None,
            paused: false,
            step_pending: false,
            view: settings.view,
            // `render` is deliberately forced on at startup whatever was saved:
            // a window that opens frozen reads as a crash.
            video: VideoOptions { render: true, ..settings.video },
            applied_render: None,
            gl_screen: None,
            gl_loaded: false,
            applied_renderer: None,
            limit_framerate: settings.limit_framerate,
            mic_static: false,
            cheats: Vec::new(),
            cheats_enabled: settings.cheats_enabled,
            applied_cheats: None,
            cheat_draft: (String::new(), String::new()),
            crash_report: None,
            font_note: String::new(),
            pads: crate::pad::Pads::new(),
            // Opened here rather than lazily: `CreationContext` already runs
            // after winit has taken the UI thread, and `Audio::spawn` puts the
            // device on a thread of its own regardless.
            audio: Audio::spawn(),
            audio_sync: settings.audio_sync,
            pause_when_unfocused: false,
            confirm_on_quit: false,
            dark_theme: settings.dark_theme,
            ui_scale: if settings.ui_scale > 0.0 { settings.ui_scale } else { 1.0 },
            save_dir: settings
                .save_dir
                .or_else(|| Some(crate::config::instance_data_dir(1, "saves"))),
            state_dir: settings
                .state_dir
                .or_else(|| Some(crate::config::instance_data_dir(1, "states"))),
            clock: crate::emu::utc_clock(),
            clock_note: String::new(),
            ram_search: RamSearch::default(),
            recents: settings.recents,
            panes: settings.open_panes,
            second_window: false,
            airwaves: Airwaves::new(),
            guest: None,
            lan_pending: None,
            lan_rom: None,
            lan_stats: None,
            lan_pace: None,
            lan_tuning: settings.lan,
            mode: Mode::Local,
            remote_host: None,
            remote_client: None,
            remote_pending: None,
            remote_tuning: settings.remote,
            remote_stats: None,
            bindings: settings.bindings.clone(),
            listening: None,
            dialog: None,
            // The environment variable still wins, because it is what a
            // scripted two-machine test sets; otherwise the address is the last
            // one that was typed, read back out of `settings.json`.
            lan_guest_address: std::env::var("MELON_EGUI_LAN_ADDR")
                .unwrap_or_else(|_| settings.lan_host_address.clone()),
            lan_bind_address: settings.lan_bind_address.clone(),
            lan_status: "LAN room is offline".to_owned(),
            lan_room: "No LAN room".to_owned(),
            instance2_settings,
            guest_textures: None,
            guest_bottom: None,
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
        if let Ok(audio) = &mut app.audio {
            audio.volume = settings.volume;
        }
        if let Some((renderer, scale)) = renderer {
            app.video.renderer = renderer;
            app.video.internal_scale = scale;
        }
        // eframe's GL context is current on this thread here, which is what
        // both of these need: glad binds against whatever is current, and the
        // shader has to be created in the context that will draw it.
        if let Some(gl) = &cc.gl {
            app.gl_loaded = melonds::gl_load(None);
            match melonds::gl_info() {
                // Which context was bound decides which renderers can work at
                // all: a driver can bind and still be too old for melonDS's
                // shaders, and that failure otherwise looks like a bug here.
                Some(info) => eprintln!("melon_egui: OpenGL bound: {info}"),
                None => eprintln!(
                    "melon_egui: could not bind OpenGL for this context, so the OpenGL \
                     renderers are unavailable and the software rasteriser is used."
                ),
            }
            match gl_screen::Screen::new(gl) {
                Ok(screen) => app.gl_screen = Some(std::sync::Arc::new(screen)),
                Err(e) => eprintln!("melon_egui: no GL blitter ({e}); OpenGL renderer disabled"),
            }
        }

        // Logged at startup because a missing sound card is otherwise only
        // visible if the user opens Config > Audio settings.
        eprintln!("melon_egui: {}", app.audio_status());
        // Before the theme, so the first frame drawn already has it: a ROM
        // title in kana is otherwise a row of boxes until something else
        // rebuilds the font atlas.
        app.font_note = match crate::fonts::install(&cc.egui_ctx) {
            Some(path) => format!("CJK fallback: {}", path.display()),
            None => "No CJK font found; Japanese text will show as boxes.".to_owned(),
        };
        eprintln!("melon_egui: {}", app.font_note);
        app.set_theme(&cc.egui_ctx, app.dark_theme);
        if settings.ui_scale > 0.0 {
            cc.egui_ctx.set_zoom_factor(settings.ui_scale);
        }
        match rom {
            Some(rom) => app.load(&rom),
            None => app.post("no cart loaded — File ▸ Open ROM..."),
        }
        // `--mp`, which only means anything once a cart is loaded.
        if launch_second && app.is_loaded() {
            app.launch_instance();
        }
        app
    }

    /// Record `rom` at the top of the recent list and save.
    pub(crate) fn push_recent(&mut self, rom: &Path) {
        let mut settings = self.settings();
        settings.push_recent(rom);
        self.recents = settings.recents.clone();
        settings.save();
    }

    /// Collect everything worth remembering and write it out.
    pub(crate) fn persist(&self) {
        self.settings().save();

        // The translation templates are written at startup instead — see
        // `crate::config::ensure_instance_layout`. A front end killed by the
        // task manager never reaches here, and a template nobody can find is
        // no better than none.
    }

    /// Everything worth remembering, gathered up.
    pub(crate) fn settings(&self) -> Settings {
        Settings {
            recents: self.recents.clone(),
            view: self.view,
            video: self.video,
            open_panes: self.panes.clone(),
            limit_framerate: self.limit_framerate,
            audio_sync: self.audio_sync,
            cheats_enabled: self.cheats_enabled,
            volume: self.volume(),
            state_dir: self.state_dir.clone(),
            save_dir: self.save_dir.clone(),
            ui_scale: self.ui_scale,
            dark_theme: self.dark_theme,
            language: self.language,
            // Remembered so a VPN address, which nobody has memorised, is typed
            // once rather than once per session.
            lan_host_address: self.lan_guest_address.clone(),
            lan_bind_address: self.lan_bind_address.clone(),
            lan: self.lan_tuning,
            remote: self.remote_tuning,
            bindings: self.bindings.clone(),
        }
    }

    /// Apply persisted settings to the currently active UI runtime.
    pub(crate) fn apply_runtime_settings(&mut self, settings: &Settings, instance: u32) {
        self.view = settings.view;
        self.video = VideoOptions { render: true, ..settings.video };
        self.limit_framerate = settings.limit_framerate;
        self.audio_sync = settings.audio_sync;
        self.cheats_enabled = settings.cheats_enabled;
        self.dark_theme = settings.dark_theme;
        self.ui_scale = if settings.ui_scale > 0.0 { settings.ui_scale } else { 1.0 };
        self.save_dir = settings
            .save_dir
            .clone()
            .or_else(|| Some(crate::config::instance_data_dir(instance, "saves")));
        self.state_dir = settings
            .state_dir
            .clone()
            .or_else(|| Some(crate::config::instance_data_dir(instance, "states")));
        self.recents = settings.recents.clone();
        self.panes = settings.open_panes.clone();
        self.lan_tuning = settings.lan;
        self.remote_tuning = settings.remote;
        self.bindings = settings.bindings.clone();
        self.set_language(settings.language);
        if let Ok(audio) = &mut self.audio {
            audio.volume = settings.volume;
        }
    }

    // -- state the menu and the panes ask about -----------------------------

    /// Switch the UI's language.
    ///
    /// A lookup, not a load: every language's strings were read once at
    /// startup ([`crate::i18n::Translations`]). That matters because the two
    /// consoles may be set to different languages, and `guest_view` applies
    /// each console's settings once per repaint — a load here would be two file
    /// reads and two map rebuilds per frame for as long as the second window is
    /// open.
    pub fn set_language(&mut self, language: crate::i18n::Language) {
        self.language = language;
    }

    /// The strings for the language currently in force.
    #[must_use]
    pub fn i18n(&self) -> &crate::i18n::I18nMap {
        self.translations.get(self.language)
    }

    /// Write the settings out, for a pane that changed one.
    pub fn save_settings(&self) {
        self.persist();
    }

    /// Boot `rom`, replacing whatever was running.
    pub(crate) fn load(&mut self, rom: &Path) {
        crate::config::ensure_instance_layout();
        // Dropped first so the outgoing cart's save is flushed before the
        // incoming one can be handed the same file.
        self.emu = None;
        self.drop_link();
        self.undo_state = None;
        self.textures = None;
        self.frames_run = 0;
        self.applied_render = None;
        self.applied_renderer = None;
        // Console 0 takes its seat on the airwaves here, at boot, rather than
        // when a second instance is launched: a console's `Host` is fixed when
        // the core is constructed, so one booted without a seat can never join
        // afterwards — its frames vanish and its peer hears silence, which is
        // what local play failing looked like. A seat costs nothing until the
        // cart calls `MP_Begin`.
        match Emu::boot_mp(
            rom,
            self.save_dir.as_ref(),
            self.state_dir.as_ref(),
            0,
            self.airwaves.client(0),
        ) {
            Ok(emu) => {
                self.emu = Some(emu);
                self.cheats = cheats::load(&Self::cheat_path(rom)).unwrap_or_default();
                self.applied_cheats = None;
                if !self.cheats.is_empty() {
                    // Worth saying out loud: a code file found beside the ROM
                    // changes what the console does, and a run that picked one
                    // up silently is a run nobody can explain afterwards.
                    let on = self.cheats.iter().filter(|cheat| cheat.enabled).count();
                    eprintln!(
                        "melon_egui: {} cheat codes from {}, {on} enabled, engine {}",
                        self.cheats.len(),
                        Self::cheat_path(rom).display(),
                        if self.cheats_enabled { "on" } else { "off" }
                    );
                }
                self.push_recent(rom);
                self.paused = false;
                self.frame_debt = 0.0;
                self.last_tick = Instant::now();
                self.post(format!("loaded {}", rom.display()));
            }
            Err(e) => self.post(format!("failed to load {}: {e}", rom.display())),
        }
    }
}
