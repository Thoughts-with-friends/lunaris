//! The front end's state, and what it does with it.
//!
//! [`MelonEgui`] is the one object the whole program is: the console, the
//! settings, the windows that are open, the link that is up. The `impl` blocks
//! beside this file are split by *what the methods are for* — booting, pacing
//! the core, a LAN session — and the other two halves live elsewhere:
//!
//! * [`crate::ui`] draws it and reports what was clicked.
//! * [`crate::file`] carries out whatever reaches the disk.
//!
//! [`MelonEgui::apply`] is the seam between them: a menu returns an
//! [`Action`], and that is where one becomes work.

pub(crate) use std::{
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};

pub(crate) use egui::{Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, pos2};
pub(crate) use melonds::{SCREEN_HEIGHT, SCREEN_WIDTH};

pub(crate) use crate::{
    audio::Audio,
    emu::Emu,
    file::{
        mch::{self, Cheat},
        settings::Settings,
    },
    gl_screen,
    mp::Airwaves,
    ui::{
        menu::{self, Action},
        notice::{Notice, Severity},
        panes,
        view::{self, Rotation, ScreenSizing, ViewOptions},
    },
    upscale,
    video::VideoOptions,
};

mod boot;
mod commands;
mod emulation;
mod host_bridge;
mod instances;
mod lan_session;
mod net_address;
mod query;
mod ram_search;
mod remote_session;

pub(crate) use gl_screen::FULL_CLIP;
pub(crate) use host_bridge::{ArcHost, LanConnection};
pub(crate) use net_address::{parse_lan_address, parse_remote_address};

pub(crate) use crate::ui::{
    layout::{guest_viewport_id, paint_screen, to_image, touch_coords},
    window::WindowConfig,
};

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
pub(crate) enum RemoteSession {
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
/// which time the menu that opened it is long gone. See [`crate::file::picker`].
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
    Directory(crate::ui::panes::PathSetting),
}

/// The DS video frame rate: `33_513_982 / 560_190` Hz. Slightly under the 60 Hz
/// a display usually runs at, so pacing has to come from a clock rather than
/// from one frame per repaint.
pub(crate) const FRAME_RATE: f64 = 59.826_1;

/// How many emulated frames a single repaint may run to catch up. A window that
/// was dragged or occluded can leave an arbitrarily large debt; running all of
/// it would make the picture lurch, so the surplus is dropped instead.
pub(crate) const MAX_CATCH_UP: u32 = 4;

/// Emulated frames per repaint while a `--shot` capture is pending, or while the
/// framerate limiter is off. Large enough to be much faster than real time,
/// small enough that the window still pumps its event loop in between.
pub(crate) const UNLIMITED_BURST: u32 = 64;

/// How often pending backup memory is written to disk.
const SAVE_FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// How many of a renderer's shaders to build before giving up on it.
///
/// Only the compute renderer compiles lazily, and it has 33; the ceiling is
/// there so a driver that never reports itself finished cannot hang the window
/// instead of falling back.
const SHADER_COMPILE_LIMIT: u32 = 256;

/// How long an OSD message stays up.
pub(crate) const OSD_LIFETIME: Duration = Duration::from_secs(3);

/// Room the menu bar takes, in points. Used to size the window so that the
/// screens land on an exact scale.
pub(crate) const CHROME_HEIGHT: f32 = 26.0;

/// Numbered savestate slots, as melonDS offers.
pub const STATE_SLOTS: u8 = 8;

pub use crate::{
    file::settings::RECENT_LIMIT,
    ui::panes::{Pane, RamSearch},
};

/// The size the window opens at: both screens at 2x, which is legible without
/// filling a modern display.
pub fn default_window_size() -> [f32; 2] {
    view::window_size_for_scale(2.0, &ViewOptions::default(), CHROME_HEIGHT).into()
}

pub struct MelonEgui {
    pub(crate) emu: Option<Emu>,

    /// Every language's strings, read once at startup.
    pub translations: crate::i18n::Translations,

    /// Uploaded once per emulated frame; `[top, bottom]`.
    pub(crate) textures: Option<[TextureHandle; 2]>,
    pub(crate) paused: bool,
    /// One frame is owed even though the core is paused — the Frame step
    /// command, which is the only way to advance while stopped.
    pub(crate) step_pending: bool,
    pub view: ViewOptions,
    pub video: VideoOptions,
    /// What was last handed to the core's render knobs, so they are only poked
    /// when something actually changed.
    pub(crate) applied_render: Option<(bool, u8)>,
    /// The blitter for the OpenGL renderer's output, once its shader has built.
    pub(crate) gl_screen: Option<gl_screen::Shared>,
    /// Whether glad has bound GL for eframe's context.
    pub(crate) gl_loaded: bool,
    /// The render settings the core was last given, so it is only poked when
    /// something actually changed — a renderer swap reallocates every render
    /// target, and even a scale change is not free.
    pub(crate) applied_renderer: Option<melonds::RenderSettings>,
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
    pub(crate) applied_cheats: Option<(bool, Vec<Cheat>)>,
    /// The Cheat codes dialog's name/text boxes, kept here so the pane itself
    /// stays a function of the app rather than owning state of its own.
    pub cheat_draft: (String, String),
    /// Which system font is filling in for the characters egui's own fonts
    /// cannot draw, for the Interface pane.
    pub font_note: Notice,
    /// What the last stopped console left behind: the reason, the state of the
    /// airwaves, and the tail of the core's own log. Shown in a pane and
    /// written to a file, because a console that stops has to explain itself
    /// to someone who is not watching a terminal.
    pub crash_report: Option<String>,
    /// The host's game controllers, merged into the keyboard's key mask.
    pub(crate) pads: crate::pad::Pads,
    /// The output stream, or the reason there is none.
    pub(crate) audio: Result<Audio, String>,
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
    pub clock_note: Notice,
    pub ram_search: RamSearch,
    /// Recently opened ROMs, newest first.
    pub(crate) recents: Vec<PathBuf>,
    /// Which auxiliary windows are open.
    pub(crate) panes: Vec<Pane>,
    /// Whether the second view of this console is open.
    pub(crate) second_window: bool,
    /// The shared wireless medium every console here sits on.
    pub airwaves: Airwaves,
    /// The second console, when "Launch new instance" has opened one. It is a
    /// separate DS on the same airwaves, not another view of the first.
    /// The second console, which runs on a thread of its own — see
    /// [`crate::guest`] for why local wireless play requires that.
    pub(crate) guest: Option<crate::guest::Guest>,
    /// A LAN host or guest connection being established off the UI thread.
    pub(crate) lan_pending: Option<Receiver<Result<LanConnection, String>>>,
    pub(crate) lan_rom: Option<PathBuf>,
    /// Reads the live link counters, once a link is up.
    pub(crate) lan_stats: Option<Box<dyn Fn() -> crate::lan::LinkStats + Send>>,
    /// The frame rate the link can sustain. Present only while a LAN game is
    /// running, which is the only time emulation is paced by anything but the
    /// wall clock.
    pub(crate) lan_pace: Option<crate::lan::LinkPace>,
    /// How the LAN transport behaves on a slow link, as the Wireless pane sets
    /// it. Read when a connection is started, so a change applies to the next
    /// link rather than the one already up.
    pub lan_tuning: crate::lan::Tuning,
    /// Which of the three things this window is.
    pub mode: Mode,
    /// The Remote Desktop session this window is the host of, if it is one.
    /// Shared with the second console's thread, which is where the encoding
    /// happens.
    pub(crate) remote_host: Option<std::sync::Arc<crate::remote::RemoteHost>>,
    /// The Remote Desktop session this window is the client of, if it is one.
    pub(crate) remote_client: Option<crate::remote::RemoteClient>,
    /// A Remote Desktop session being established off the UI thread.
    pub(crate) remote_pending: Option<Receiver<Result<RemoteSession, String>>>,
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
    pub lan_status: Notice,
    pub lan_room: String,
    /// Settings persisted independently for the second console.
    pub(crate) instance2_settings: Settings,
    pub(crate) guest_textures: Option<[TextureHandle; 2]>,
    /// Where the guest's bottom screen was drawn, for its own touch input.
    pub(crate) guest_bottom: Option<Rect>,
    /// Whether each screen had anything on it last frame, which is what
    /// `ScreenSizing::Auto` decides on.
    pub(crate) screens_live: [bool; 2],
    /// Fractional emulated frames owed, carried across repaints so a 60 Hz
    /// display does not slowly outrun the DS's 59.83 Hz.
    pub(crate) frame_debt: f64,
    pub(crate) last_tick: Instant,
    pub(crate) last_save_flush: Instant,
    /// Where the bottom screen was drawn last repaint. Touch is sampled before
    /// this repaint's layout runs, so it uses the previous rectangle — one
    /// repaint of latency, invisible at these sizes. `None` when the bottom
    /// screen is not shown, which makes it untouchable, as it should be.
    pub(crate) bottom_screen: Option<Rect>,
    /// Emulated frames run and the wall-clock window they took, for the
    /// throughput readout.
    pub(crate) fps_frames: u32,
    pub(crate) fps_since: Instant,
    pub(crate) fps: f64,
    /// The newest OSD message and when it was posted.
    pub(crate) osd: Option<(Notice, Instant)>,
    /// The state the console was in before the last `Load state`, so that it can
    /// be taken back.
    pub(crate) undo_state: Option<Vec<u8>>,
    /// Emulated frames run since the cart booted, for [`Self::service_shot`].
    pub(crate) frames_run: u64,
    /// Which key and which pad button each DS button answers to.
    pub bindings: crate::bindings::Bindings,
    /// The binding the Input dialog is waiting for a press to fill in.
    ///
    /// While this is set the next key — or the next pad button — is taken as
    /// the new binding rather than as a button press for the console, which is
    /// why it lives here rather than in the pane: [`Self::advance`] has to know
    /// not to hand that press to the cart.
    pub listening: Option<(crate::bindings::DsInput, crate::bindings::Device)>,

    /// The file dialog that is open, if any, and what it is asking about.
    ///
    /// One at a time: the dialogs are not parented to the window, so two open
    /// at once would be two unrelated windows with no way to tell which
    /// belonged to which command.
    pub(crate) dialog: Option<crate::file::picker::Pending<DialogPurpose>>,
    /// The UI's language and the strings that go with it.
    pub language: crate::i18n::Language,
    /// `--shot`: capture the window once this many frames have run, write it
    /// there, and quit. `None` in normal use.
    pub(crate) shot: Option<(u64, PathBuf)>,
    /// Whether the capture has already been asked for, so it is asked for once.
    pub(crate) shot_requested: bool,

    pub window: WindowConfig,
}
