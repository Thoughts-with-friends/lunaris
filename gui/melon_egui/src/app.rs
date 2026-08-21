//! The egui application: pacing the core, blitting its framebuffers, and
//! feeding it keys and touch.
//!
//! The window's shape follows melonDS's — a menu bar over the screens, with
//! messages drawn over the picture as an OSD rather than in a status bar. The
//! menu itself lives in [`crate::menu`].

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};

use egui::{Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, pos2};
use melonds::{SCREEN_HEIGHT, SCREEN_WIDTH, keys};

use crate::{
    audio::Audio,
    cheats::{self, Cheat},
    config::Settings,
    emu::Emu,
    gl_screen,
    menu::{self, Action},
    mp::Airwaves,
    panes, upscale,
    video::VideoOptions,
    view::{self, Rotation, ScreenSizing, ViewOptions},
};

/// A LAN link that finished its handshake on the connection thread, on its way
/// to being handed to a console.
///
/// Carries the link's own measurement handles alongside the transport, because
/// `Box<dyn melonds::Host>` erases them and the front end needs both: the stats
/// for the Wireless pane, and the pace for [`MelonEgui::advance`].
struct LanConnection {
    host: Box<dyn melonds::Host>,
    local_addr: String,
    remote_addr: String,
    /// Reads the live link counters. `None` would mean a transport with no
    /// measurement, which this front end no longer has.
    stats: Box<dyn Fn() -> crate::lan::LinkStats + Send>,
    pace: crate::lan::LinkPace,
}

/// Lets a link be both the console's `Host` and the pane's counter source.
///
/// `Nds::new` takes ownership of a `Box<dyn Host>`, but the Wireless pane has
/// to keep reading the same link's counters for as long as it is up. Sharing
/// the transport behind an `Arc` is the whole of the trick; every method simply
/// forwards.
struct ArcHost<T>(std::sync::Arc<T>);

impl<T: melonds::Host + Sync> melonds::Host for ArcHost<T> {
    fn write_save(&self, data: &[u8], writeoffset: u32, writelen: u32) {
        self.0.write_save(data, writeoffset, writelen);
    }

    fn signal_stop(&self, reason: i32) {
        self.0.signal_stop(reason);
    }

    fn mp_begin(&self) {
        self.0.mp_begin();
    }

    fn mp_end(&self) {
        self.0.mp_end();
    }

    fn mp_send_packet(&self, data: &[u8], timestamp: u64) -> i32 {
        self.0.mp_send_packet(data, timestamp)
    }

    fn mp_recv_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        self.0.mp_recv_packet(data, now, timestamp)
    }

    fn mp_send_cmd(&self, data: &[u8], timestamp: u64) -> i32 {
        self.0.mp_send_cmd(data, timestamp)
    }

    fn mp_send_reply(&self, data: &[u8], timestamp: u64, aid: u16) -> i32 {
        self.0.mp_send_reply(data, timestamp, aid)
    }

    fn mp_send_ack(&self, data: &[u8], timestamp: u64) -> i32 {
        self.0.mp_send_ack(data, timestamp)
    }

    fn mp_recv_host_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        self.0.mp_recv_host_packet(data, now, timestamp)
    }

    fn mp_recv_replies(&self, data: &mut [u8], now: u64, timestamp: u64, aidmask: u16) -> u16 {
        self.0.mp_recv_replies(data, now, timestamp, aidmask)
    }

    fn mp_clock(&self, now: u64) {
        self.0.mp_clock(now);
    }
}

/// Where a Remote Desktop session's other end is.
///
/// The port comes from [`crate::remote::Tuning::port`] and **replaces** whatever
/// the address field carries, rather than merely filling in for a missing one.
/// The address boxes are shared with LAN mode, so they usually hold that mode's
/// port; honouring it here would silently point Remote Desktop at the LAN
/// listener. One box on the pane deciding the port for this mode is easier to
/// reason about than two fields that have to agree.
fn parse_remote_address(text: &str, port: u16) -> Result<std::net::SocketAddr, String> {
    let ip = text
        .parse::<std::net::SocketAddr>()
        .map(|addr| addr.ip())
        .or_else(|_| text.parse::<std::net::IpAddr>())
        .map_err(|error| format!("invalid Remote Desktop address {text}: {error}"))?;
    Ok(std::net::SocketAddr::new(ip, port))
}

#[cfg(test)]
mod remote_address_tests {
    use super::parse_remote_address;

    /// The tuning's port wins, so the LAN boxes' port cannot misdirect a
    /// Remote Desktop session.
    #[test]
    fn the_tuned_port_replaces_whatever_the_field_holds() {
        assert_eq!(
            parse_remote_address("192.168.1.20:7064", 7065).unwrap().to_string(),
            "192.168.1.20:7065"
        );
        assert_eq!(
            parse_remote_address("192.168.1.20", 7065).unwrap().to_string(),
            "192.168.1.20:7065"
        );
        assert_eq!(parse_remote_address("0.0.0.0:1", 9000).unwrap().to_string(), "0.0.0.0:9000");
        assert!(parse_remote_address("not an address", 7065).is_err());
    }
}

fn parse_lan_address(text: &str, default_port: u16) -> Result<std::net::SocketAddr, String> {
    text.parse::<std::net::SocketAddr>()
        .or_else(|_| {
            text.parse::<std::net::IpAddr>().map(|ip| std::net::SocketAddr::new(ip, default_port))
        })
        .map_err(|error| format!("invalid LAN address {text}: {error}"))
}

#[cfg(test)]
mod lan_address_tests {
    use super::parse_lan_address;

    #[test]
    fn plain_ip_uses_the_default_lan_port() {
        assert_eq!(
            parse_lan_address("192.168.1.20", 7064).unwrap().to_string(),
            "192.168.1.20:7064"
        );
    }

    #[test]
    fn explicit_port_is_preserved() {
        assert_eq!(
            parse_lan_address("192.168.1.20:8000", 7064).unwrap().to_string(),
            "192.168.1.20:8000"
        );
    }
}

/// Which of the three things this window is.
///
/// A type rather than a scattering of `is_some()` checks, for the reason
/// [`DialogPurpose`] is a type: several places have to agree about it — whether
/// a cart is loaded, whether to repaint, which menu entries do anything — and a
/// `match` that must be exhaustive is what keeps them agreeing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// An ordinary console, possibly with a second one beside it.
    #[default]
    Local,
    /// Both consoles run here; the second one's picture and sound go out over
    /// the network and its controls come back. See [`crate::remote`].
    RemoteHost,
    /// No emulation at all: a screen and a pair of speakers for a console
    /// running somewhere else.
    ///
    /// **Owns nothing.** Saves, savestates, cheats and instance directories all
    /// belong to the host, which is the whole point — a client that stored
    /// anything would be a second copy of the save to keep in step.
    RemoteClient,
}

impl Mode {
    /// Whether this window runs an emulator of its own.
    #[must_use]
    pub const fn emulates(self) -> bool {
        matches!(self, Self::Local | Self::RemoteHost)
    }
}

/// A Remote Desktop session being established off the UI thread.
enum RemoteSession {
    // Boxed because the two ends are very different sizes — the host carries an
    // encoder with a whole frame of reference pixels in it — and this value
    // only ever travels once, down a channel, on the way to being unwrapped.
    Host(Box<crate::remote::RemoteHost>),
    Client(Box<crate::remote::RemoteClient>),
}

/// What an open file dialog is asking about.
///
/// Held alongside the dialog so that its answer cannot be applied to the wrong
/// command — the dialog is answered several repaints after it was opened, by
/// which time the menu that opened it is long gone. See [`crate::fs`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogPurpose {
    /// A cart to boot.
    OpenRom,
    /// A `.sav` to write into the cart's backup memory.
    ImportSave,
    /// Where to write a savestate.
    SaveState,
    /// A savestate to read.
    LoadState,
    /// A `.mch` cheat file to merge in.
    ImportCheats,
    /// Which console the answer is for: the second one, for each of the above.
    /// Kept as separate variants rather than a flag so that a missing case is a
    /// compile error rather than a command that quietly drives the wrong
    /// console — which is the bug this whole mechanism exists to prevent.
    GuestImportSave,
    GuestSaveState,
    GuestLoadState,
    /// A directory for one of the Path settings.
    Directory(crate::panes::PathSetting),
}

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

/// How many of a renderer's shaders to build before giving up on it.
///
/// Only the compute renderer compiles lazily, and it has 33; the ceiling is
/// there so a driver that never reports itself finished cannot hang the window
/// instead of falling back.
const SHADER_COMPILE_LIMIT: u32 = 256;

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

    /// Every language's strings, read once at startup.
    pub translations: crate::i18n::Translations,

    /// Uploaded once per emulated frame; `[top, bottom]`.
    textures: Option<[TextureHandle; 2]>,
    paused: bool,
    /// One frame is owed even though the core is paused — the Frame step
    /// command, which is the only way to advance while stopped.
    step_pending: bool,
    pub view: ViewOptions,
    pub video: VideoOptions,
    /// What was last handed to the core's render knobs, so they are only poked
    /// when something actually changed.
    applied_render: Option<(bool, u8)>,
    /// The blitter for the OpenGL renderer's output, once its shader has built.
    gl_screen: Option<gl_screen::Shared>,
    /// Whether glad has bound GL for eframe's context.
    gl_loaded: bool,
    /// The render settings the core was last given, so it is only poked when
    /// something actually changed — a renderer swap reallocates every render
    /// target, and even a scale change is not free.
    applied_renderer: Option<melonds::RenderSettings>,
    /// Whether to pace the core against wall-clock time. Off, it runs as fast
    /// as it can, matching melonDS's "Limit framerate".
    pub limit_framerate: bool,
    /// White noise on the microphone, the only mic input this build has.
    pub mic_static: bool,
    /// The cart's Action Replay codes, read from its `.mch` when it booted.
    /// Held whether or not cheats are on, so the master switch loses nothing.
    pub cheats: Vec<Cheat>,
    /// melonDS's "Enable cheats". Off, the core is handed an empty list, which
    /// is what makes cheats cost nothing at all rather than merely do nothing.
    pub cheats_enabled: bool,
    /// What was last handed to the core, so the list is only pushed on a
    /// change: it is copied into the console each time.
    applied_cheats: Option<(bool, Vec<Cheat>)>,
    /// The Cheat codes dialog's name/text boxes, kept here so the pane itself
    /// stays a function of the app rather than owning state of its own.
    pub cheat_draft: (String, String),
    /// Which system font is filling in for the characters egui's own fonts
    /// cannot draw, for the Interface pane.
    pub font_note: String,
    /// What the last stopped console left behind: the reason, the state of the
    /// airwaves, and the tail of the core's own log. Shown in a pane and
    /// written to a file, because a console that stops has to explain itself
    /// to someone who is not watching a terminal.
    pub crash_report: Option<String>,
    /// The host's game controllers, merged into the keyboard's key mask.
    pads: crate::pad::Pads,
    /// The output stream, or the reason there is none.
    audio: Result<Audio, String>,
    /// Pace emulation against the sound card rather than the clock, so audio
    /// plays without gaps even if the two clocks disagree slightly.
    pub audio_sync: bool,
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
    /// The shared wireless medium every console here sits on.
    pub airwaves: Airwaves,
    /// The second console, when "Launch new instance" has opened one. It is a
    /// separate DS on the same airwaves, not another view of the first.
    /// The second console, which runs on a thread of its own — see
    /// [`crate::guest`] for why local wireless play requires that.
    guest: Option<crate::guest::Guest>,
    /// A LAN host or guest connection being established off the UI thread.
    lan_pending: Option<Receiver<Result<LanConnection, String>>>,
    lan_rom: Option<PathBuf>,
    /// Reads the live link counters, once a link is up.
    lan_stats: Option<Box<dyn Fn() -> crate::lan::LinkStats + Send>>,
    /// The frame rate the link can sustain. Present only while a LAN game is
    /// running, which is the only time emulation is paced by anything but the
    /// wall clock.
    lan_pace: Option<crate::lan::LinkPace>,
    /// How the LAN transport behaves on a slow link, as the Wireless pane sets
    /// it. Read when a connection is started, so a change applies to the next
    /// link rather than the one already up.
    pub lan_tuning: crate::lan::Tuning,
    /// Which of the three things this window is.
    pub mode: Mode,
    /// The Remote Desktop session this window is the host of, if it is one.
    /// Shared with the second console's thread, which is where the encoding
    /// happens.
    remote_host: Option<std::sync::Arc<crate::remote::RemoteHost>>,
    /// The Remote Desktop session this window is the client of, if it is one.
    remote_client: Option<crate::remote::RemoteClient>,
    /// A Remote Desktop session being established off the UI thread.
    remote_pending: Option<Receiver<Result<RemoteSession, String>>>,
    /// How Remote Desktop behaves, as the Wireless pane sets it.
    pub remote_tuning: crate::remote::Tuning,
    /// What the live Remote Desktop session is doing, sampled each repaint so
    /// the pane and the menu read one consistent set of numbers.
    pub remote_stats: Option<crate::remote::RemoteStats>,
    /// Guest's editable LAN host address.
    pub lan_guest_address: String,
    /// Host's editable local bind address.
    pub lan_bind_address: String,
    /// Human-readable LAN room and connection status.
    pub lan_status: String,
    pub lan_room: String,
    /// Settings persisted independently for the second console.
    instance2_settings: Settings,
    guest_textures: Option<[TextureHandle; 2]>,
    /// Where the guest's bottom screen was drawn, for its own touch input.
    guest_bottom: Option<Rect>,
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
    /// The file dialog that is open, if any, and what it is asking about.
    ///
    /// One at a time: the dialogs are not parented to the window, so two open
    /// at once would be two unrelated windows with no way to tell which
    /// belonged to which command.
    dialog: Option<crate::fs::Pending<DialogPurpose>>,
    /// The UI's language and the strings that go with it.
    pub language: crate::i18n::Language,
    /// `--shot`: capture the window once this many frames have run, write it
    /// there, and quit. `None` in normal use.
    shot: Option<(u64, PathBuf)>,
    /// Whether the capture has already been asked for, so it is asked for once.
    shot_requested: bool,
}

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
    fn push_recent(&mut self, rom: &Path) {
        let mut settings = self.settings();
        settings.push_recent(rom);
        self.recents = settings.recents.clone();
        settings.save();
    }

    /// Collect everything worth remembering and write it out.
    fn persist(&self) {
        self.settings().save();

        // The translation templates are written at startup instead — see
        // `crate::config::ensure_instance_layout`. A front end killed by the
        // task manager never reaches here, and a template nobody can find is
        // no better than none.
    }

    /// Everything worth remembering, gathered up.
    fn settings(&self) -> Settings {
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
        }
    }

    /// Apply persisted settings to the currently active UI runtime.
    fn apply_runtime_settings(&mut self, settings: &Settings, instance: u32) {
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
        self.set_language(settings.language);
        if let Ok(audio) = &mut self.audio {
            audio.volume = settings.volume;
        }
    }

    // -- state the menu and the panes ask about -----------------------------

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

    /// The ROM info pane's rows, or `None` with no cart loaded.
    /// Where a cart's codes live in instance1's dedicated cheat directory.
    /// The file keeps melonDS's `.mch` format so both front ends can use it.
    pub fn cheat_path(rom: &Path) -> PathBuf {
        crate::config::instance_data_dir(1, "cheats").join(rom.file_stem().map_or_else(
            || PathBuf::from("cheats.mch"),
            |name| PathBuf::from(format!("{}.mch", name.to_string_lossy())),
        ))
    }

    /// The path the running cart's codes are read from and written to.
    pub fn cheat_file(&self) -> Option<PathBuf> {
        self.emu.as_ref().map(|emu| Self::cheat_path(&emu.rom_path))
    }

    /// Turn the dialog's two boxes into a code, reporting a bad paste rather
    /// than adding something the engine would read as garbage.
    pub fn add_cheat_from_draft(&mut self) {
        let (name, text) = self.cheat_draft.clone();
        match cheats::parse_code(&text) {
            Ok(code) if code.is_empty() => self.post("no code words in that text"),
            Ok(code) => {
                let odd = !code.len().is_multiple_of(2);
                self.cheats.push(Cheat {
                    name: if name.trim().is_empty() { "Unnamed".to_owned() } else { name },
                    code,
                    enabled: true,
                    ..Cheat::default()
                });
                self.cheat_draft = (String::new(), String::new());
                if odd {
                    self.post("added, but that code has an odd number of words");
                }
            }
            Err(token) => self.post(format!("not a 32-bit hex word: {token}")),
        }
    }

    /// Write the current list back to the cart's `.mch`.
    pub fn save_cheats(&mut self) {
        let Some(path) = self.cheat_file() else { return };
        match cheats::save(&path, &self.cheats) {
            Ok(()) => self.post(format!("cheats written to {}", path.display())),
            Err(e) => self.post(e),
        }
    }

    /// Read a `.mch` the user picked, replacing the list.
    pub fn import_cheats(&mut self, path: &Path) {
        match cheats::load(path) {
            Ok(list) => {
                let count = list.len();
                self.cheats = list;
                self.post(format!("{count} codes read from {}", path.display()));
            }
            Err(e) => self.post(e),
        }
    }

    /// Hand the core the codes it should be running.
    ///
    /// Only on a change: the list is copied into the console, and it is pushed
    /// from the same place every repaint so that a code toggled in the dialog
    /// takes effect on the next frame.
    fn apply_cheats(&mut self) {
        let wanted = (self.cheats_enabled, self.cheats.clone());
        if self.applied_cheats.as_ref() == Some(&wanted) || self.emu.is_none() {
            return;
        }
        let Some(emu) = &mut self.emu else { return };
        // Cheats off is an empty list rather than a flag: melonDS runs whatever
        // is in the console's list, so this is the only way to stop it.
        let installed: Vec<melonds::Cheat> = if self.cheats_enabled {
            self.cheats.iter().map(Cheat::to_core).collect()
        } else {
            Vec::new()
        };
        emu.nds.set_cheats(&installed);
        // The second console is a second cart, and a cheat that is on for one
        // player and off for the other desynchronises a linked game outright.
        // Its own `.mch` was read when it booted; this is the master switch and
        // any code edited since, which have to reach both.
        if let Some(guest) = &self.guest {
            guest.send(crate::guest::Command::SetCheats(installed.clone()));
        }
        self.applied_cheats = Some(wanted);
    }

    /// Gather everything that might explain a stopped console, show it, and
    /// write it beside the executable.
    ///
    /// Written to a file because the usual way to run this is by launching the
    /// executable, which on Windows has no console attached: a diagnostic that
    /// only reaches stderr reaches nobody. The pane opens by itself for the
    /// same reason.
    fn write_crash_report(&mut self, who: &str, note: &str) {
        let mut report = format!("melon_egui: {who} {note}\n");
        if let Some(emu) = &mut self.emu {
            report.push_str(&format!(
                "cart: {} [{}]\nframes run: console 0 = {}",
                emu.info.title,
                emu.info.gamecode,
                emu.nds.frame_count()
            ));
        }
        if let Some(guest) = &self.guest {
            report.push_str(&format!(", second instance = {}", guest.frame_count()));
        }
        report.push('\n');

        // Who was on the air, and what they had exchanged: local play failing
        // shows up here as one side sending and nothing coming back.
        let connected = self.airwaves.connected();
        for (i, counters) in self.airwaves.counters().iter().enumerate().take(2) {
            report.push_str(&format!(
                "console {i}: {} | sent {}/{} cmd/reply, generic {}, ack {} | \
                 received cmd {}, reply {}, generic {} | stale replies {} | \
                 wifi clock {} | last reply mask {:04X}\n",
                if connected.get(i) == Some(&true) { "on the air" } else { "not on the air" },
                counters.sent_cmd,
                counters.sent_reply,
                counters.sent_generic,
                counters.sent_ack,
                counters.recv_cmd,
                counters.recv_reply,
                counters.recv_generic,
                counters.stale_replies,
                counters.clock,
                counters.last_reply_mask,
            ));
        }

        report.push_str("\n-- the last of the wireless traffic ------------------\n");
        let log = self.airwaves.log();
        for event in log.iter().rev().take(40).rev() {
            report.push_str(&format!(
                "console {} {} len={} ts={}\n",
                event.sender,
                event.kind.label(),
                event.len,
                event.timestamp
            ));
        }

        report.push_str("\n-- the core's own last words -------------------------\n");
        for line in crate::logger::recent() {
            report.push_str(&line);
            report.push('\n');
        }

        let path = crate::config::config_dir().join("last-stop.txt");
        match std::fs::create_dir_all(crate::config::config_dir())
            .and_then(|()| std::fs::write(&path, &report))
        {
            Ok(()) => {
                eprintln!("melon_egui: wrote {}", path.display());
                self.post(format!("stop report written to {}", path.display()));
            }
            Err(e) => eprintln!("melon_egui: could not write {}: {e}", path.display()),
        }
        self.crash_report = Some(report);
        if !self.panes.contains(&panes::Pane::Crash) {
            self.panes.push(panes::Pane::Crash);
        }
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

    /// Open a system file dialog, off the UI thread.
    ///
    /// Refuses while one is already open rather than stacking two unparented
    /// dialogs the user cannot tell apart. See [`crate::fs`] for why this is not
    /// a blocking call.
    fn ask(&mut self, purpose: DialogPurpose, request: crate::fs::Request) {
        if self.dialog.is_some() {
            self.post("a file dialog is already open");
            return;
        }
        match crate::fs::Pending::spawn(purpose, request) {
            Ok(pending) => self.dialog = Some(pending),
            // Reported rather than swallowed: from the user's side a dialog
            // that never appears is a menu entry that was ignored.
            Err(error) => self.post(error),
        }
    }

    /// Act on a dialog the user has finished with.
    ///
    /// Called once per repaint from [`Self::advance`], which is what keeps the
    /// window drawing and the console running while a dialog is on screen.
    fn poll_dialog(&mut self) {
        let Some((purpose, path)) = crate::fs::Pending::take_answer(&mut self.dialog) else {
            return;
        };
        // Cancelled. Not worth an OSD message: the user knows they cancelled.
        let Some(path) = path else { return };
        match purpose {
            DialogPurpose::OpenRom => self.load(&path),
            DialogPurpose::ImportSave => self.import_savefile_from(&path),
            DialogPurpose::SaveState => self.write_state_to(&path),
            DialogPurpose::LoadState => self.read_state_from(&path),
            DialogPurpose::ImportCheats => self.import_cheats(&path),
            DialogPurpose::GuestImportSave => match std::fs::read(&path) {
                Ok(data) => self.command_guest(crate::guest::Command::ImportSave(data)),
                Err(error) => self.post(format!("cannot read {}: {error}", path.display())),
            },
            DialogPurpose::GuestSaveState => {
                self.command_guest(crate::guest::Command::SaveState(None, Some(path)));
            }
            DialogPurpose::GuestLoadState => {
                self.command_guest(crate::guest::Command::LoadState(None, Some(path)));
            }
            DialogPurpose::Directory(setting) => {
                setting.set(self, path);
                self.persist();
            }
        }
    }

    /// Where a dialog for `extension` should open: this instance's own
    /// directory when there is one, so a savestate dialog lands in `states`
    /// rather than wherever the system last was.
    fn dialog_dir(&self, kind: &str) -> Option<PathBuf> {
        match kind {
            "saves" => self.save_dir.clone(),
            "states" => self.state_dir.clone(),
            _ => Some(crate::config::instance_data_dir(1, kind)),
        }
    }

    /// Hand a command to the second console, if there is one.
    pub fn command_guest(&mut self, command: crate::guest::Command) {
        match &self.guest {
            Some(guest) => guest.send(command),
            None => self.post("no second console is running"),
        }
    }

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

    /// Ask for a `.mch` cheat file to merge into the current cart's list.
    pub fn ask_for_cheat_file(&mut self) {
        self.ask(
            DialogPurpose::ImportCheats,
            crate::fs::Request::open("Open melonDS cheats")
                .filter("melonDS cheats", &["mch"])
                .directory(self.dialog_dir("cheats")),
        );
    }

    /// Ask for a directory for one of the Path settings.
    pub fn ask_for_directory(&mut self, setting: crate::panes::PathSetting) {
        self.ask(
            DialogPurpose::Directory(setting),
            crate::fs::Request::folder("Choose a directory")
                .directory(Some(crate::config::instances_dir())),
        );
    }

    /// Write the settings out, for a pane that changed one.
    pub fn save_settings(&self) {
        self.persist();
    }

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

    /// Whether a Remote Desktop session is up or being established.
    #[must_use]
    pub const fn remote_running(&self) -> bool {
        self.remote_host.is_some() || self.remote_client.is_some() || self.remote_pending.is_some()
    }

    /// What the live LAN link is doing, for the Wireless pane.
    #[must_use]
    pub fn lan_stats(&self) -> Option<crate::lan::LinkStats> {
        self.lan_stats.as_ref().map(|read| read())
    }

    /// Recompute the throughput readout from the frames counted since the last
    /// window closed.
    ///
    /// Split out so a Remote Desktop client — which counts *received* frames
    /// rather than emulated ones — reports its rate the same way, and the
    /// number in the corner means "frames a second on this screen" in both
    /// modes.
    fn report_fps(&mut self) {
        let elapsed = self.fps_since.elapsed();
        if elapsed >= Duration::from_millis(500) {
            self.fps = f64::from(self.fps_frames) / elapsed.as_secs_f64();
            self.fps_frames = 0;
            self.fps_since = Instant::now();
        }
    }

    /// Post an OSD message. Also where every command reports its outcome, so
    /// that failures are visible without a console.
    fn post(&mut self, message: impl Into<String>) {
        let message = message.into();
        eprintln!("melon_egui: {message}");
        self.osd = Some((message, Instant::now()));
    }

    /// Boot `rom`, replacing whatever was running.
    fn load(&mut self, rom: &Path) {
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

    /// Start a LAN host or guest connection without blocking the UI thread.
    fn start_lan(&mut self, host: bool) {
        if self.lan_pending.is_some() {
            self.post("a LAN connection is already being established");
            return;
        }
        let Some(rom) = self.emu.as_ref().map(|emu| emu.rom_path.clone()) else {
            self.post("load a cart first");
            return;
        };
        self.emu = None;
        self.drop_link();
        self.textures = None;
        self.undo_state = None;
        self.lan_rom = Some(rom);
        let (sender, receiver) = std::sync::mpsc::channel();
        let bind = self.lan_bind_address.clone();
        let address = self.lan_guest_address.clone();
        let bind_for_thread = bind.clone();
        let address_for_thread = address.clone();
        // Read once, here, so that a link keeps whatever tuning it was started
        // with even if the pane is edited while it runs — the two ends have to
        // agree about nothing, but a budget that changes underneath a round in
        // flight is needlessly confusing to reason about.
        let tuning = self.lan_tuning;
        let spawned = std::thread::Builder::new()
            .name(if host { "melon-egui-lan-host" } else { "melon-egui-lan-guest" }.to_owned())
            .spawn(move || {
                let result = if host {
                    parse_lan_address(&bind_for_thread, 7064).and_then(|addr| {
                        crate::lan::LanHost::accept(addr, tuning)
                            .and_then(|transport| {
                                let local_addr = transport.local_addr()?;
                                let remote_addr = transport.remote_addr().to_string();
                                let pace = transport.pace();
                                let transport = std::sync::Arc::new(transport);
                                let reader = std::sync::Arc::clone(&transport);
                                Ok(LanConnection {
                                    local_addr: local_addr.to_string(),
                                    remote_addr,
                                    stats: Box::new(move || reader.stats()),
                                    pace,
                                    host: Box::new(ArcHost(transport)),
                                })
                            })
                            .map_err(|e| format!("LAN host failed: {e}"))
                    })
                } else {
                    let local = "0.0.0.0:0";
                    local.parse().map_err(|e| format!("invalid LAN client address: {e}")).and_then(
                        |local| {
                            parse_lan_address(&address_for_thread, 7064).and_then(|remote| {
                                crate::lan::LanGuest::connect(local, remote, tuning)
                                    .and_then(|transport| {
                                        let local_addr = transport.local_addr()?;
                                        let pace = transport.pace();
                                        let transport = std::sync::Arc::new(transport);
                                        let reader = std::sync::Arc::clone(&transport);
                                        Ok(LanConnection {
                                            local_addr: local_addr.to_string(),
                                            remote_addr: remote.to_string(),
                                            stats: Box::new(move || reader.stats()),
                                            pace,
                                            host: Box::new(ArcHost(transport)),
                                        })
                                    })
                                    .map_err(|e| format!("LAN guest failed: {e}"))
                            })
                        },
                    )
                };
                let _ = sender.send(result);
            })
            .map_err(|e| format!("cannot start LAN connection: {e}"));
        if let Err(error) = spawned {
            self.lan_rom = None;
            self.post(error);
            return;
        }
        self.lan_pending = Some(receiver);
        // Saved on the attempt rather than on success: an address that did not
        // answer is still the one the user meant to type, and having to type it
        // again to retry is the annoyance this exists to remove.
        self.persist();
        self.lan_room = if host { "Hosting LAN room" } else { "Joining LAN room" }.to_owned();
        self.lan_status = if host {
            format!("Checking: waiting for guest on {bind}")
        } else {
            format!("Checking: connecting to {address}")
        };
        self.post(if host {
            format!("waiting for a LAN guest on {bind}")
        } else {
            format!("connecting to LAN host {address}")
        });
    }

    /// Begin a Remote Desktop session, without blocking the UI thread.
    ///
    /// As host: both consoles will run here, and the second one's picture and
    /// sound go out to whoever connects. As client: this window stops being an
    /// emulator and becomes a screen.
    fn start_remote(&mut self, host: bool) {
        if self.remote_pending.is_some() {
            self.post("a Remote Desktop session is already being established");
            return;
        }
        if host && !self.is_loaded() {
            self.post("load a cart first — the host runs both consoles");
            return;
        }
        let tuning = self.remote_tuning;
        let bind = self.lan_bind_address.clone();
        let address = self.lan_guest_address.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name(
                if host { "melon-egui-remote-host" } else { "melon-egui-remote-client" }.to_owned(),
            )
            .spawn(move || {
                let result = if host {
                    parse_remote_address(&bind, tuning.port).and_then(|addr| {
                        crate::remote::RemoteHost::accept(addr, tuning)
                            .map(|host| RemoteSession::Host(Box::new(host)))
                            .map_err(|error| format!("Remote Desktop host failed: {error}"))
                    })
                } else {
                    parse_remote_address(&address, tuning.port).and_then(|remote| {
                        // Any local port: the client only ever talks to the one
                        // host, which answers wherever the hello came from.
                        let local = std::net::SocketAddr::from(([0, 0, 0, 0], 0));
                        crate::remote::RemoteClient::connect(local, remote, tuning)
                            .map(|client| RemoteSession::Client(Box::new(client)))
                            .map_err(|error| format!("Remote Desktop client failed: {error}"))
                    })
                };
                let _ = sender.send(result);
            })
            .map_err(|error| format!("cannot start a Remote Desktop session: {error}"));
        if let Err(error) = spawned {
            self.post(error);
            return;
        }
        self.remote_pending = Some(receiver);
        // Saved on the attempt: an address that did not answer is still the one
        // the user meant to type.
        self.persist();
        self.lan_room =
            if host { "Remote Desktop: hosting" } else { "Remote Desktop: joining" }.to_owned();
        // The port shown is the one that will actually be used — see
        // `parse_remote_address`.
        self.lan_status = if host {
            format!(
                "waiting for a client on {}",
                parse_remote_address(&self.lan_bind_address, self.remote_tuning.port)
                    .map_or_else(|error| error, |addr| addr.to_string())
            )
        } else {
            format!(
                "connecting to {}",
                parse_remote_address(&self.lan_guest_address, self.remote_tuning.port)
                    .map_or_else(|error| error, |addr| addr.to_string())
            )
        };
        let message = self.lan_status.clone();
        self.post(message);
    }

    /// Finish a Remote Desktop session that the connection thread established.
    fn poll_remote(&mut self) {
        // Sampled every repaint so the pane and the menu agree, and so a
        // session that has gone quiet is visible rather than merely stale.
        self.remote_stats = match (&self.remote_host, &self.remote_client) {
            (Some(host), _) => Some(host.stats()),
            (_, Some(client)) => Some(client.stats()),
            _ => None,
        };

        let Some(receiver) = &self.remote_pending else { return };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.remote_pending = None;
                self.post("the Remote Desktop worker stopped unexpectedly");
                return;
            }
        };
        self.remote_pending = None;
        match result {
            Ok(RemoteSession::Host(host)) => {
                let host = *host;
                let local = host.local_addr().map_or_else(|_| "?".to_owned(), |a| a.to_string());
                let remote = host.remote_addr();
                self.remote_host = Some(std::sync::Arc::new(host));
                self.mode = Mode::RemoteHost;
                // The remote player's console. Launched *after* the session
                // exists, because the stream is fixed when the thread starts.
                self.close_guest();
                self.launch_instance();
                self.lan_room = "Remote Desktop: hosting".to_owned();
                self.lan_status = format!("Client {remote} connected; listening on {local}");
                self.post(format!("Remote Desktop: {remote} is playing instance 2"));
            }
            Ok(RemoteSession::Client(client)) => {
                let client = *client;
                // A client emulates nothing, so whatever was running here stops
                // — and its save is flushed on the way out.
                self.emu = None;
                self.drop_link();
                self.close_guest();
                self.textures = None;
                let remote = client.remote_addr();
                self.remote_client = Some(client);
                self.mode = Mode::RemoteClient;
                self.paused = false;
                let local = self
                    .remote_client
                    .as_ref()
                    .and_then(|client| client.local_addr().ok())
                    .map_or_else(|| "?".to_owned(), |addr| addr.to_string());
                self.lan_room = "Remote Desktop: connected".to_owned();
                self.lan_status = format!("Watching {remote} from {local}");
                self.post(format!("Remote Desktop: connected to {remote}"));
            }
            Err(error) => {
                self.lan_room = "Remote Desktop: offline".to_owned();
                self.lan_status = error.clone();
                self.post(error);
            }
        }
    }

    /// End a Remote Desktop session and go back to being an ordinary window.
    fn stop_remote(&mut self) {
        if self.remote_host.is_none() && self.remote_client.is_none() {
            self.post("no Remote Desktop session is running");
            return;
        }
        // The host's second console was the remote player's; it goes with them.
        self.close_guest();
        self.remote_host = None;
        self.remote_client = None;
        self.remote_stats = None;
        self.textures = None;
        self.mode = Mode::Local;
        self.lan_room = "Remote Desktop: offline".to_owned();
        self.lan_status = "No Remote Desktop session".to_owned();
        self.post("Remote Desktop session ended");
    }

    /// Close the second console, if one is open.
    fn close_guest(&mut self) {
        self.guest = None;
        self.guest_textures = None;
    }

    /// Show the picture and play the sound a host is sending.
    ///
    /// Everything a client does in place of emulating: there is no core here,
    /// so the textures are filled from the decoder and the audio ring from the
    /// network rather than from an [`Emu`].
    fn service_remote_client(&mut self, ctx: &egui::Context) {
        let Some(client) = &self.remote_client else { return };

        if let Some([top, bottom]) = client.take_screens() {
            let filter =
                if self.view.filtering { TextureOptions::LINEAR } else { TextureOptions::NEAREST };
            let images = [
                to_image(&top, self.video.upscale, self.video.upscale_factor()),
                to_image(&bottom, self.video.upscale, self.video.upscale_factor()),
            ];
            match &mut self.textures {
                Some(textures) => {
                    for (texture, image) in textures.iter_mut().zip(images) {
                        texture.set(image, filter);
                    }
                }
                None => {
                    let [t, b] = images;
                    self.textures = Some([
                        ctx.load_texture("remote-top", t, filter),
                        ctx.load_texture("remote-bottom", b, filter),
                    ]);
                }
            }
            self.screens_live = [true, true];
            self.frames_run += 1;
            self.fps_frames += 1;
        }

        let samples = client.take_audio();
        if let (Ok(audio), false) = (&mut self.audio, samples.is_empty()) {
            audio.push(&samples);
        }

        // The controls, sent every repaint whatever the player is doing — see
        // `crate::remote::Input`.
        let keys = ctx.input(|i| {
            BINDINGS
                .iter()
                .filter(|(key, ..)| i.key_down(*key))
                .fold(0, |mask, (_, bit, _)| mask | bit)
        }) | self.pads.poll();
        let touch = self.sample_touch(ctx);
        client.send_input(keys, touch);
    }

    /// Forget the LAN link the last console was on.
    ///
    /// Both handles have to go, and for two different reasons.
    ///
    /// `lan_pace` is the one that bites: [`crate::lan::LinkPace`] is only
    /// updated from inside `mp_recv_replies`, so once the console that made
    /// those calls is gone the last value it wrote **freezes**. A LAN game over
    /// a 100 ms link leaves it at about 10 fps; stopping that game and opening
    /// an ordinary cart would then run the new console at 10 fps for the rest
    /// of the session, with nothing on screen to explain why.
    ///
    /// `lan_stats` holds an `Arc` on the transport, so leaving it behind also
    /// keeps the link's receive and probe threads alive — pinging a peer that
    /// is no longer there — and leaves the Wireless pane reporting a dead
    /// link's counters as though they were live.
    fn drop_link(&mut self) {
        self.lan_stats = None;
        self.lan_pace = None;
    }

    /// Finish a background LAN connection and boot the current cart on it.
    fn poll_lan(&mut self) {
        let Some(receiver) = &self.lan_pending else { return };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.lan_pending = None;
                self.post("LAN connection worker stopped unexpectedly");
                return;
            }
        };
        self.lan_pending = None;
        let Some(rom) = self.lan_rom.take() else {
            self.post("LAN connected, but no cart is loaded");
            return;
        };
        match result.and_then(|connection| {
            let local_addr = connection.local_addr.clone();
            let remote_addr = connection.remote_addr.clone();
            let LanConnection { host, stats, pace, .. } = connection;
            Emu::boot_lan(&rom, self.save_dir.as_ref(), self.state_dir.as_ref(), host)
                .map(|emu| (emu, local_addr, remote_addr, stats, pace))
        }) {
            Ok((emu, local_addr, remote_addr, stats, pace)) => {
                self.emu = Some(emu);
                self.lan_stats = Some(stats);
                self.lan_pace = Some(pace);
                self.cheats = cheats::load(&Self::cheat_path(&rom)).unwrap_or_default();
                self.applied_cheats = None;
                self.paused = false;
                self.frame_debt = 0.0;
                self.last_tick = Instant::now();
                self.lan_status = format!("Connected: local {local_addr}, remote {remote_addr}");
                self.lan_room = "LAN room connected".to_owned();
                self.post(format!("LAN game connected: {}", rom.display()));
            }
            Err(error) => {
                self.lan_stats = None;
                self.lan_pace = None;
                self.lan_status = format!("Connection check failed: {error}");
                self.lan_room = "LAN room offline".to_owned();
                self.post(format!("LAN game failed: {error}"));
            }
        }
    }

    /// Perform a menu action for the second console rather than the first.
    ///
    /// The second console's window draws the same menu bar, and until this
    /// existed every entry in it acted on the *first* console — which is why
    /// only the entries that happen to be pure UI appeared to work there. What
    /// cannot be done for the second console (opening a different cart, LAN,
    /// launching a third) says so instead of silently doing it to the first.
    fn apply_to_guest(&mut self, action: Action) {
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
    fn apply_ui_only(&mut self, action: Action) {
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

    /// Open or close one of the auxiliary windows.
    pub fn toggle_pane(&mut self, pane: Pane) {
        if let Some(at) = self.panes.iter().position(|open| *open == pane) {
            self.panes.remove(at);
        } else {
            self.panes.push(pane);
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
    fn apply_as_client(&mut self, action: Action, ctx: &egui::Context) {
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

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
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

    /// Where the second console's backup memory goes: `instance2/` under
    /// console 0's save directory, seeded with a copy of console 0's file so
    /// the two start from the same progress and then diverge, as two carts do.
    ///
    /// `None` if the directory cannot be made, which falls back to sharing —
    /// worse, but better than refusing to launch.
    fn guest_save_dir(&mut self, rom: &Path) -> Option<PathBuf> {
        let host_save = Settings::redirect(self.save_dir.as_ref(), rom, "sav");
        let dir = crate::config::instance_data_dir(2, "saves");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.post(format!("cannot make {}: {e}; sharing the save", dir.display()));
            return None;
        }
        let guest_save = Settings::redirect(Some(&dir), rom, "sav");
        if !guest_save.exists()
            && host_save.exists()
            && let Err(e) = std::fs::copy(&host_save, &guest_save)
        {
            self.post(format!("cannot seed {}: {e}", guest_save.display()));
        }
        Some(dir)
    }

    /// Open a second console on the same cart and the same airwaves, which is
    /// what makes local wireless play testable in one window.
    ///
    /// melonDS launches a whole second process for this; here it is a second
    /// [`Emu`] driven from the same repaint, which keeps the two wifi clocks in
    /// step without any cross-process synchronisation.
    fn launch_instance(&mut self) {
        if self.guest.is_some() {
            self.guest = None;
            self.guest_textures = None;
            self.post("second instance closed");
            return;
        }
        let Some(rom) = self.emu.as_ref().map(|emu| emu.rom_path.clone()) else {
            self.post("load a cart first");
            return;
        };
        // Console 0 already holds seat 0 (see `load`), so only the second
        // console is booted here.
        //
        // It gets a save directory of its own, seeded from console 0's file:
        // two consoles are two carts, and pointing both at one `.sav` means
        // whichever writes last wins -- with a real save on the line.
        let save_dir = self.guest_save_dir(&rom);
        // Put the newcomer on console 0's wireless timebase. melonDS starts a
        // console's wifi clock at `frames * 16716` when wifi powers on, so a
        // console booted mid-session would stamp its frames however far behind
        // it started -- minutes, here -- and the two would read each other's
        // traffic as ancient.
        let start_frame = self.emu.as_mut().map_or(0, |host| host.nds.frame_count());
        // In Remote Desktop mode this console is the remote player's: its
        // picture and sound go out over the session, and its controls come back
        // from it. See `crate::remote`.
        let stream = self.remote_host.clone();
        let streamed = stream.is_some();
        self.guest = Some(crate::guest::Guest::spawn(
            &rom,
            save_dir,
            Some(crate::config::instance_data_dir(2, "states")),
            Some(crate::config::instance_data_dir(2, "cheats")),
            1,
            self.airwaves.client(1),
            start_frame,
            stream,
        ));
        self.post(if streamed {
            "second instance launched — its picture and sound go to the remote player"
        } else {
            "second instance launched - both consoles share the airwaves"
        });
    }

    /// Show this front end's directory in the system file manager.
    fn open_directory(&mut self) {
        self.open_instance_directory(1);
    }

    /// Show one instance's directory, so the second console's window opens its
    /// own `saves`/`states`/`cheats` rather than the first console's.
    fn open_instance_directory(&mut self, instance: u32) {
        let dir = crate::config::instance_dir(instance);
        self.reveal(&dir);
    }

    /// Ask for a save file to write into the cart's backup memory.
    fn import_savefile(&mut self) {
        self.ask(
            DialogPurpose::ImportSave,
            crate::fs::Request::open("Import a save file")
                .filter("save file", &["sav", "dsv", "bin"])
                .directory(self.dialog_dir("saves")),
        );
    }

    /// Perform the import the dialog asked about.
    fn import_savefile_from(&mut self, path: &Path) {
        let outcome = std::fs::read(path)
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

    /// A numbered slot writes straight away; "File..." asks first and lands in
    /// [`Self::write_state_to`] once the dialog is answered.
    fn save_state(&mut self, slot: Option<u8>) {
        let Some(emu) = &mut self.emu else { return };
        let Some(slot) = slot else {
            // Pre-filled with the cart's own name, so "File..." does not open
            // on an empty name box beside eight slots that are named for you.
            let suggestion = emu
                .rom_path
                .file_stem()
                .map_or_else(|| "state".to_owned(), |stem| stem.to_string_lossy().into_owned());
            let directory = self.dialog_dir("states");
            return self.ask(
                DialogPurpose::SaveState,
                crate::fs::Request::save("Save state")
                    .filter("savestate", &["ml1"])
                    .file_name(format!("{suggestion}.ml1"))
                    .directory(directory),
            );
        };
        let path = emu.state_path(slot);
        self.write_state_to(&path);
    }

    fn write_state_to(&mut self, path: &Path) {
        let Some(emu) = &mut self.emu else { return };
        let mut buf = Vec::new();
        let outcome = emu.nds.save_state(&mut buf).map_err(|e| e.to_string()).and_then(|()| {
            std::fs::write(path, &buf).map_err(|e| format!("cannot write {}: {e}", path.display()))
        });
        match outcome {
            Ok(()) => {
                let mib = buf.len() as f64 / (1024.0 * 1024.0);
                self.post(format!("state saved to {} ({mib:.1} MiB)", path.display()));
            }
            Err(e) => self.post(format!("save state failed: {e}")),
        }
    }

    /// As [`Self::save_state`]: a slot acts at once, "File..." asks first.
    fn load_state(&mut self, slot: Option<u8>) {
        let Some(emu) = &mut self.emu else { return };
        let Some(slot) = slot else {
            return self.ask(
                DialogPurpose::LoadState,
                crate::fs::Request::open("Load state")
                    .filter("savestate", &["ml1"])
                    .directory(self.dialog_dir("states")),
            );
        };
        let path = emu.state_path(slot);
        self.read_state_from(&path);
    }

    fn read_state_from(&mut self, path: &Path) {
        let Some(emu) = &mut self.emu else { return };

        // Snapshot first: a load with nothing to go back to is a load that
        // cannot be undone, and melonDS offers exactly that undo.
        let mut before = Vec::new();
        let snapshot = emu.nds.save_state(&mut before).is_ok();

        let outcome = std::fs::read(path)
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
        self.poll_lan();
        self.poll_remote();
        // Before the early return: a dialog may be the only thing that will
        // produce a cart to run.
        self.poll_dialog();

        // A client has no console to advance: its picture arrives over the
        // network and its input goes back the same way. Everything below this
        // point is about running an emulator, which is precisely what a client
        // does not do.
        if self.mode == Mode::RemoteClient {
            self.service_remote_client(ctx);
            self.report_fps();
            return;
        }

        // Pumped before the early return: gilrs only notices a pad being
        // plugged in while its queue is drained, and the Input pane lists
        // controllers whether or not a cart is running.
        let pad_keys = self.pads.poll();

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
        } else if let Some((at, _)) = &self.shot {
            // A capture has to land on an exact frame to be worth comparing
            // against another run, so the burst is cut short at the target and
            // the console stops there — it must not keep running while the
            // screenshot makes its way back from the GPU.
            UNLIMITED_BURST.min(at.saturating_sub(self.frames_run) as u32)
        } else if !self.limit_framerate {
            UNLIMITED_BURST
        } else {
            // Ordinarily the DS's own rate. On a LAN link it is instead what
            // the link can sustain: `mp_recv_replies` blocks inside a frame for
            // as long as a reply takes to come back, so a console on a slow
            // link genuinely cannot issue 59.83 frames a second. Pacing to the
            // native rate anyway only builds a debt that is discharged as a
            // burst of rounds, which floods the peer — the very thing that
            // turns a slow link into a broken one.
            let rate = self.lan_pace.as_ref().map_or(FRAME_RATE, crate::lan::LinkPace::frame_rate);
            self.frame_debt += elapsed.as_secs_f64() * rate;
            let due = (self.frame_debt as u32).min(MAX_CATCH_UP);
            self.frame_debt -= f64::from(due);
            // Whatever is still owed after the cap is dropped rather than
            // carried, so a stall cannot turn into a burst later.
            if self.frame_debt > f64::from(MAX_CATCH_UP) {
                self.frame_debt = 0.0;
            }
            due
        };

        // "Audio sync": when the ring is already comfortably full the sound
        // card has not caught up, so this repaint runs nothing and lets it. Only
        // meaningful while the framerate limiter is on -- fast-forward
        // deliberately outruns the device.
        let due = if self.audio_sync && self.limit_framerate && !self.paused {
            match &self.audio {
                Ok(audio) if audio.fill() > 0.75 => 0,
                _ => due,
            }
        } else {
            due
        };

        // Sampled before the core is borrowed, since both readings come out of
        // `self` and the emulator borrow would conflict with them.
        let keys = ctx.input(|i| {
            BINDINGS
                .iter()
                .filter(|(key, ..)| i.key_down(*key))
                .fold(0, |mask, (_, bit, _)| mask | bit)
            // Pads are merged with the keyboard rather than replacing it, so
            // neither has to be chosen up front and holding both is one press.
        }) | pad_keys;
        let touch = self.sample_touch(ctx);
        // The guest window has its own input; see `guest_view`.
        let (guest_keys, guest_touch) = self.sample_guest_input(ctx);

        let mut ran = 0;
        let mut stopped = false;
        // The core's account of why, which only exists for as long as it takes
        // to read it out of the console that gave it.
        let mut stop_note = None;
        self.apply_render_settings();
        self.apply_renderer();
        self.apply_cheats();
        let mic_static = self.mic_static;

        // The second console runs on its own thread; all that reaches it from
        // here is the input its window collected.
        if let Some(guest) = &self.guest {
            guest.set_input(guest_keys, guest_touch);
            guest.set_paused(self.paused || unfocused);
        }

        let Self { emu, .. } = self;

        if let Some(emu) = emu.as_mut() {
            emu.nds.set_keys(keys);
            emu.set_mic_static(mic_static);
            match touch {
                Some((x, y)) => emu.nds.touch(x, y),
                None => emu.nds.release_screen(),
            }
        }
        //
        // Whether a console is still running is *asked*, not inferred from the
        // frame's scanline count. A console asleep — which is a state a cart
        // enters deliberately, and which the wireless code passes through —
        // draws no scanlines at all, and reading that as "stopped" is what used
        // to freeze a perfectly healthy console the moment it slept.
        for _ in 0..due {
            if let Some(emu) = emu.as_mut() {
                emu.nds.run_frame();
                if !emu.nds.is_running() {
                    stop_note = emu.stop_reason();
                    stopped = true;
                    break;
                }
                ran += 1;
            }
        }

        // Whatever the second console's thread has to say. Its own stop is
        // reported the same way the first console's is, but it is closed rather
        // than paused: a console that has stopped is not going to draw again.
        if let Some(note) = self.guest.as_ref().and_then(crate::guest::Guest::take_note) {
            self.post(format!("second instance {note}"));
            if self.guest.as_ref().is_some_and(crate::guest::Guest::finished) {
                self.write_crash_report("second instance", &note);
            }
        }
        if self.guest.as_ref().is_some_and(crate::guest::Guest::finished) {
            self.guest = None;
            self.guest_textures = None;
        }
        if stopped {
            let note = stop_note.unwrap_or_else(|| "stopped".to_owned());
            eprintln!("melon_egui: console {note}");
            self.post(format!("console {note}"));
            self.write_crash_report("console 0", &note);
        }

        if ran > 0 {
            self.drain_audio();
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
            // The second console keeps its own `.sav` under `instance2/saves`,
            // and a window closed by the task manager would otherwise lose
            // whatever it had not written.
            if let Some(guest) = &self.guest {
                guest.send(crate::guest::Command::FlushSave);
            }
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

    /// Push the Video settings' renderer half into the core.
    ///
    /// Only on a change: swapping renderer reallocates every render target,
    /// and even a change of internal resolution is not free.
    fn apply_renderer(&mut self) {
        use crate::video::Renderer;

        // Asking for OpenGL without a working blitter would leave a console
        // rendering into a texture nothing can draw, so that choice is taken
        // back here rather than in the core.
        if self.video.renderer.is_gl() && !self.gl_available() {
            self.video.renderer = Renderer::Software;
        }
        let wanted = self.video.to_core();
        if self.applied_renderer == Some(wanted) || self.emu.is_none() {
            return;
        }

        let Some(emu) = &mut self.emu else { return };
        let installed = Renderer::from_core(emu.set_render_settings(wanted));
        // The compute renderer builds its shaders on demand; without this the
        // first frames come out empty.
        let mut compiled = 0;
        while emu.gl_shader_compile_step().is_some() && compiled < SHADER_COMPILE_LIMIT {
            compiled += 1;
        }

        self.applied_renderer = Some(wanted);
        if installed.is_gl() {
            // The CPU textures stop being refreshed under OpenGL and would
            // otherwise linger as a stale picture.
            self.textures = None;
        }
        if installed == self.video.renderer {
            self.post(match installed {
                Renderer::Software => format!(
                    "renderer: software{}",
                    if self.video.threaded_software { ", threaded" } else { "" }
                ),
                _ => format!(
                    "renderer: {} at {}x internal resolution",
                    installed.label(),
                    self.video.scale()
                ),
            });
        } else {
            // melonDS installs a software renderer of its own when the one it
            // was handed cannot initialise, so this is what actually happened
            // rather than what was asked for.
            self.post(format!(
                "could not create the {} renderer; on {} instead",
                self.video.renderer.label(),
                installed.label()
            ));
            self.video.renderer = installed;
            self.applied_renderer = Some(self.video.to_core());
        }
    }

    fn apply_render_settings(&mut self) {
        let (top, bottom) = match self.view.sizing {
            ScreenSizing::TopOnly => (!self.view.swap, self.view.swap),
            ScreenSizing::BottomOnly => (self.view.swap, !self.view.swap),
            _ => (true, true),
        };
        let wanted = (self.video.render, self.video.displayed_mask(top, bottom));
        if self.applied_render == Some(wanted) {
            return;
        }
        if let Some(emu) = &mut self.emu {
            emu.set_render(wanted.0);
            emu.set_displayed_screens(wanted.1);
            self.applied_render = Some(wanted);
        }
    }

    /// Move whatever the SPU has produced into the output ring.
    ///
    /// Called once per batch of emulated frames rather than per frame, since the
    /// core buffers internally and the ring is what actually paces playback.
    fn drain_audio(&mut self) {
        let (Ok(audio), Some(emu)) = (&mut self.audio, &mut self.emu) else {
            return;
        };
        let queued = emu.nds.audio_queued();
        if queued == 0 {
            return;
        }
        // Interleaved stereo, so two `i16` per sample frame.
        let mut buf = vec![0i16; queued * 2];
        let read = emu.nds.read_audio(&mut buf);
        audio.push(&buf[..read * 2]);
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
    ///
    /// Nothing to do under an OpenGL renderer: its picture never leaves the
    /// GPU, and is drawn by [`Self::screens`] straight from the texture.
    fn upload(&mut self, ctx: &egui::Context) {
        let filter =
            if self.view.filtering { TextureOptions::LINEAR } else { TextureOptions::NEAREST };
        let Some(emu) = &mut self.emu else {
            return;
        };
        if emu.gl_output().is_some() {
            // `ScreenSizing::Auto` decides on whether a screen has anything on
            // it, which cannot be sampled from a texture without reading it
            // back every frame. Both screens count as live instead, which is
            // what Auto resolves to whenever it cannot tell.
            self.screens_live = [true, true];
            return;
        }
        let Some((top, bottom)) = emu.nds.framebuffers() else {
            return;
        };
        // What `ScreenSizing::Auto` decides on: a screen showing nothing but
        // black is one the console is not really using.
        let lit = |fb: &[u32]| fb.iter().any(|&px| px & 0x00FF_FFFF != 0);
        let live = [lit(top), lit(bottom)];
        let (method, factor) = (self.video.upscale, self.video.upscale_factor());
        let images = [to_image(top, method, factor), to_image(bottom, method, factor)];
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

        // Under OpenGL the picture is a texture in eframe's context rather than
        // CPU pixels, so it is drawn by a callback inside the GL painter.
        if let Some(output) = self.emu.as_mut().and_then(Emu::gl_output)
            && let Some(screen) = self.gl_screen.clone()
        {
            let filter =
                if self.view.filtering { eframe::glow::LINEAR } else { eframe::glow::NEAREST };
            for (rect, layer) in [(placed.top, 0.0f32), (placed.bottom, 1.0f32)] {
                let Some(rect) = rect else { continue };
                let screen = screen.clone();
                let callback = egui_glow::CallbackFn::new(move |_info, painter| {
                    // egui_glow sets the GL viewport to this callback's own
                    // rectangle before calling it, so the quad just covers clip
                    // space and lands exactly where the layout put the screen.
                    screen.paint(painter.gl(), output.texture, FULL_CLIP, layer, filter);
                });
                ui.painter()
                    .add(egui::PaintCallback { rect, callback: std::sync::Arc::new(callback) });
            }
            return;
        }

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
            // Without this the window looks hung rather than deliberately still.
            let frozen = if self.video.render { "" } else { "  [rendering off]" };
            lines.insert(0, format!("{:.1} FPS{paused}{frozen}", self.fps));
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

    /// Keys and touch for the second console, read from its own viewport.
    ///
    /// egui keeps a separate input state per viewport, so this reads the guest
    /// window's rather than the main window's — otherwise one keypress would
    /// drive both consoles.
    fn sample_guest_input(&self, ctx: &egui::Context) -> (u32, Option<(u16, u16)>) {
        if self.guest.is_none() {
            return (0, None);
        }
        let id = guest_viewport_id();
        let read = |i: &egui::InputState| {
            let keys = BINDINGS
                .iter()
                .filter(|(key, ..)| i.key_down(*key))
                .fold(0, |mask, (_, bit, _)| mask | bit);
            let pointer = i.pointer.primary_down().then(|| i.pointer.interact_pos()).flatten();
            (keys, pointer)
        };
        // Before the viewport's first repaint this reads a default state, which
        // is simply "nothing held" -- the right answer for a window that has
        // not appeared yet.
        let (keys, pointer) = ctx.input_for(id, read);
        let touch = self
            .guest_bottom
            .zip(pointer)
            .and_then(|(rect, pos)| touch_coords(rect, pos, self.instance2_settings.view.rotation));
        (keys, touch)
    }

    /// The second console's window: its own screens, its own input.
    fn guest_view(&mut self, ctx: &egui::Context) {
        if self.guest.is_none() {
            return;
        }
        let host_settings = self.settings();
        self.apply_runtime_settings(&self.instance2_settings.clone(), 2);
        // Upload the guest's picture with the same conversion the host uses.
        let filter =
            if self.view.filtering { TextureOptions::LINEAR } else { TextureOptions::NEAREST };
        if let Some(screens) = self.guest.as_ref().and_then(crate::guest::Guest::take_screens) {
            let [top, bottom] = screens;
            let images = [
                to_image(&top, self.video.upscale, self.video.upscale_factor()),
                to_image(&bottom, self.video.upscale, self.video.upscale_factor()),
            ];
            match &mut self.guest_textures {
                Some(textures) => {
                    for (texture, image) in textures.iter_mut().zip(images) {
                        texture.set(image, filter);
                    }
                }
                None => {
                    let [t, b] = images;
                    self.guest_textures = Some([
                        ctx.load_texture("guest-top", t, filter),
                        ctx.load_texture("guest-bottom", b, filter),
                    ]);
                }
            }
        }

        let Some(textures) = self.guest_textures.clone() else {
            let updated = self.settings();
            updated.save_for(2);
            self.instance2_settings = updated;
            self.apply_runtime_settings(&host_settings, 1);
            return;
        };
        let view = self.resolved_view();
        let builder = egui::ViewportBuilder::default()
            .with_title("melon_egui - instance 2")
            .with_inner_size(default_window_size())
            // Same COM-apartment reason as the main window.
            .with_drag_and_drop(false);

        let mut closed = false;
        let mut bottom_rect = None;
        let mut action = None;
        ctx.show_viewport_immediate(guest_viewport_id(), builder, |ctx, _class| {
            ctx.set_zoom_factor(self.ui_scale);
            self.set_theme(ctx, self.dark_theme);
            egui::TopBottomPanel::top("guest-menu").show(ctx, |ui| {
                action = menu::bar(self, ui);
            });
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(
                ctx,
                |ui| {
                    let area = ui.max_rect();
                    let placed = view::layout(area, &view);
                    bottom_rect = placed.bottom;
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
            panes::show(self, ctx);
            // Resizing has to happen against *this* viewport's context, so it
            // is taken here rather than in `apply_to_guest`: sending it to the
            // main window's context would resize the wrong window.
            if let Some(Action::ScreenSize(scale)) = action {
                let size = view::window_size_for_scale(scale, &self.view, CHROME_HEIGHT);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                action = None;
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                closed = true;
            }
        });
        self.guest_bottom = bottom_rect;
        let updated = self.settings();
        updated.save_for(2);
        self.instance2_settings = updated;
        self.apply_runtime_settings(&host_settings, 1);
        ctx.set_zoom_factor(self.ui_scale);
        self.set_theme(ctx, self.dark_theme);
        // Routed to the *second* console. Before this existed the second
        // window's menu bar drove the first console, which is what "only some
        // of it works over there" was.
        if let Some(action) = action {
            self.apply_to_guest(action);
        }
        if closed {
            self.guest = None;
            self.guest_textures = None;
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
            self.shot_core_picture(path.clone());
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if !self.shot_requested && self.frames_run >= *at {
            self.shot_requested = true;
            println!("shot: {} frames run, requesting capture", self.frames_run);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }

    /// Alongside a `--shot` of the window, write the core's own picture when it
    /// is an OpenGL renderer drawing it: `<out>_core_top.png` and
    /// `<out>_core_bottom.png`, read back from the texture at the internal
    /// resolution.
    ///
    /// The window capture is at window size whatever the renderer is doing, so
    /// it cannot show that the internal resolution reached the rasteriser.
    /// These can: their pixel size *is* `256*scale x 192*scale`.
    fn shot_core_picture(&mut self, path: PathBuf) {
        let Some(emu) = &mut self.emu else { return };
        let Some(output) = emu.gl_output() else { return };

        let (w, h) = (output.width as usize, output.height as usize);
        let mut pixels = vec![0u32; w * h];
        for (screen, name) in [(0u8, "top"), (1, "bottom")] {
            if emu.gl_read_output(screen, &mut pixels) == 0 {
                eprintln!("shot: could not read the {name} screen back from the GL renderer");
                continue;
            }
            // BGRA in memory, as the software framebuffers are, so the channel
            // order here is the one `to_image` uses.
            let rgb: Vec<u8> = pixels
                .iter()
                .flat_map(|&px| [(px >> 16) as u8, (px >> 8) as u8, px as u8])
                .collect();
            let out = path.with_file_name(format!(
                "{}_core_{name}.png",
                path.file_stem().unwrap_or_default().to_string_lossy()
            ));
            match image::save_buffer(&out, &rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8)
            {
                Ok(()) => println!("shot: wrote {} ({w}x{h})", out.display()),
                Err(e) => eprintln!("shot: failed to write {}: {e}", out.display()),
            }
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
        self.guest_view(ctx);
        self.second_view(ctx);
        if let Some(action) = action {
            self.apply(action, ctx);
        }

        self.service_shot(ctx);

        // The core is paced off wall-clock time, so the window has to keep
        // repainting rather than wait for input. Paused, there is nothing to
        // redraw until something happens.
        // A dialog is answered on another thread, so the window has to keep
        // repainting to notice — that is what makes the console keep running
        // while it is open rather than freezing behind it.
        // A client repaints continuously: its picture arrives from the network
        // and its input has to leave on the same cadence, neither of which
        // egui knows to wake up for.
        if self.emu.is_some() && (!self.paused || self.step_pending)
            || self.mode == Mode::RemoteClient
            || self.lan_pending.is_some()
            || self.remote_pending.is_some()
            || self.dialog.is_some()
            || self.guest.is_some()
        {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Some(emu) = &self.emu {
            emu.flush_save();
        }
        // The blitter's program and vertex array belong to eframe's context,
        // which is still current here and gone afterwards.
        if let (Some(gl), Some(screen)) = (gl, self.gl_screen.take()) {
            screen.destroy(gl);
        }
        self.persist();
    }
}

/// A quad covering the whole GL viewport, which egui_glow has already set to
/// the paint callback's rectangle.
const FULL_CLIP: [f32; 4] = [-1.0, -1.0, 2.0, 2.0];

/// The second console's window. A stable id, so the viewport survives repaints.
fn guest_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("melon_egui-instance-2")
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
/// One screen's framebuffer as an egui image, post-processed by `method` at
/// `factor` on the way (see [`crate::upscale`]).
///
/// The core's pixels are BGRA in memory; the swizzle here is the software
/// renderer's counterpart to the one `gl_screen`'s shader does on the GPU.
fn to_image(fb: &[u32], method: upscale::Method, factor: u8) -> ColorImage {
    let rgba: Vec<u8> =
        fb.iter().flat_map(|&px| [(px >> 16) as u8, (px >> 8) as u8, px as u8, 0xFF]).collect();
    let (buf, width, height) = upscale::upscale(rgba, SCREEN_WIDTH, SCREEN_HEIGHT, method, factor);
    let pixels =
        buf.as_chunks::<4>().0.iter().map(|px| Color32::from_rgb(px[0], px[1], px[2])).collect();
    ColorImage {
        size: [width, height],
        pixels,
        source_size: egui::vec2(width as f32, height as f32),
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
        let image = to_image(&fb, crate::upscale::Method::None, 1);
        assert_eq!(image.size, [SCREEN_WIDTH, SCREEN_HEIGHT]);
        assert_eq!(image.pixels[0], Color32::from_rgb(0x12, 0x34, 0x56));
        assert_eq!(image.pixels[1], Color32::from_rgb(0xFF, 0, 0));
        assert_eq!(image.pixels[2], Color32::BLACK);
    }
}
