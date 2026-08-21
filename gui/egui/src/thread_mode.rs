//! Thread Mode: a second emulated DS running in **this** process, on its own
//! thread, in its own OS window, linked to the main instance by in-process
//! channels instead of UDP sockets.
//!
//! # Why this exists
//! Verifying local wireless play otherwise needs two `lunaris` processes, two
//! rooms, and a real network path -- and when it fails, the fault could be in
//! any of them. Thread Mode removes the sockets, the room control plane, the
//! pacing controller and the packet loss, leaving only the emulated Wi-Fi
//! hardware and the MP backend. If local play fails here, the fault is in the
//! emulator; if it works here but not over LAN, the fault is in the transport.
//! See `docs/design/review_mp_local2.md` §7.1c.
//!
//! # Why a real thread
//! [`LocalMp`] is a port of melonDS's `LocalMP`, which is built around a
//! blocking semaphore rendezvous: a host waiting for client replies blocks
//! until *the other instance's thread* posts. melonDS runs one instance per
//! thread, and this arrangement reproduces that. Stepping both instances from
//! one thread -- as `core/examples/local_mp_loopback.rs` does -- cannot satisfy
//! that rendezvous, because the only thread that could produce the reply is the
//! one blocked waiting for it.
//!
//! # Layering
//! ```text
//! UI thread                                worker thread
//!   main NDS (host)  <-- LocalMpHub -->      guest NDS (client)
//!   main viewport                                 |
//!   guest viewport ---- input snapshot ---------->+
//!                  <--- screens + diag -----------+
//! ```
//! The guest gets its own [`egui::ViewportId`], i.e. a real second OS window
//! with its own keyboard focus and its own pointer. Input collected there is
//! published into a mutex the worker samples once per emulated frame; the
//! worker publishes its screens and MP diagnostic back the same way. Nothing
//! else crosses the boundary: the guest owns its `NDS` outright.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use lunaris_gui_common::{
    config::{Config, ScreenFilter},
    framebuffer::{
        PlacementRect, ScreenLayout, abgr1555_to_rgba8, layout_screens, point_to_touch_coords,
    },
    input::enums::KeyboardKey,
    loader,
};
use nds_core::{
    NDS,
    nds::MpTransport,
    net::{LocalMp, LocalMpHub, MpInterfaceTransport},
};

use crate::input::InputState;

/// Instance index the Thread Mode guest boots as, i.e.
/// `instances/instance2/`.
///
/// Must not be [`crate::MAIN_INSTANCE`]: the index selects both the Wi-Fi MAC
/// perturbation and the whole per-instance directory tree (its own
/// `config.json`, `saves/`, `states/`, `cheats/`, `logs/`). Two instances
/// sharing a MAC can never associate with each other, and two sharing a save
/// directory would overwrite each other's progress.
pub const GUEST_INSTANCE: u8 = 1;

/// Nominal DS frame rate, used to pace the worker.
const NDS_FPS: f64 = 59.8261;

/// A menu action the guest viewport asked for, queued for the worker.
///
/// The guest's `NDS` lives on the worker thread and is never touched from the
/// UI thread, so every File/Emulation menu action becomes one of these. They
/// are fire-and-forget: anything that produces a file (exporting a save) is
/// given its destination up front and written by the worker, which avoids
/// needing a reply channel just to move bytes back across the boundary.
enum GuestCommand {
    SaveState(usize),
    LoadState(usize),
    /// Copy this file over the guest's `.sav` and reboot the instance, exactly
    /// as the main window's "Import Save" does.
    ImportSave(PathBuf),
    ExportSave(PathBuf),
    Reset,
    SetPaused(bool),
    /// Audio output level, 0..=100, mirroring the main window's Audio window.
    SetVolume(f32),
    /// Emulation speed multiplier. Also decides audio-clock pacing: only
    /// native speed can stay synchronised to the audio device.
    SetSpeed(f32),
}

/// Input the guest viewport collected, for the worker to apply.
///
/// Published as whole-state rather than as events because that is what the
/// emulator consumes: [`crate::input::apply_input_bindings`] resolves the full
/// key state once per frame, and the stylus is either down somewhere or up.
/// A dropped update therefore costs one frame of latency and can never leave a
/// key stuck down, which an event stream could.
#[derive(Default, Clone)]
pub struct GuestInput {
    /// Keyboard keys currently held while the guest viewport has focus.
    keyboard: HashSet<KeyboardKey>,
    /// Where the stylus is pressed on the guest's bottom screen, if at all.
    touch: Option<(usize, usize)>,
}

/// What the worker publishes for the guest viewport to render.
#[derive(Default)]
pub struct GuestFrame {
    /// Top and bottom screens, ABGR1555, `256 * 192` each. Empty until the
    /// guest has produced its first frame.
    screens: [Vec<u16>; 2],
    /// Emulated frames completed so far.
    frames: u64,
    /// The guest's local-multiplayer diagnostic, already rendered to text.
    diag: String,
    /// Set once the worker has stopped, so the UI can report a guest that died
    /// rather than showing a frozen last frame forever.
    finished: bool,
    /// Result of the most recent [`GuestCommand`], for the status bar. The
    /// worker is the only place that knows whether a save actually landed.
    status: String,
    /// Whether the guest is currently paused.
    paused: bool,
}

/// Owns the Thread Mode guest: its worker thread, the shared local-MP hub, the
/// frame it publishes, and the input it consumes.
pub struct ThreadMode {
    hub: Option<Arc<LocalMpHub>>,
    worker: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    frame: Arc<Mutex<GuestFrame>>,
    input: Arc<Mutex<GuestInput>>,
    commands: Arc<Mutex<Vec<GuestCommand>>>,
    /// The host's wireless timebase, republished every repaint.
    ///
    /// The guest reads it whenever it boots — at start, and again on Reset or
    /// Import Save, which rebuild the console from scratch. A console whose
    /// uptime restarts has to be told where its peer's clock has got to, or it
    /// starts stamping its frames from zero again. See
    /// [`NDS::wifi_clock_reference`].
    clock: Arc<AtomicU64>,
    /// Shown in the UI when [`ThreadMode::start`] could not boot the guest.
    last_error: Option<String>,
    /// Whether the control window is open. The guest's own viewport is tied to
    /// the worker's lifetime instead, so closing this panel never silently
    /// kills a running instance.
    pub is_open: bool,
    textures: Option<[egui::TextureHandle; 2]>,
    /// Where the guest's bottom screen was drawn last repaint, for mapping a
    /// pointer position to touch coordinates.
    bottom_placement: Option<(egui::Pos2, PlacementRect)>,
    /// Mirrors the process-wide association trace flag. See
    /// [`nds_core::net::set_assoc_trace`].
    assoc_trace: bool,
    /// The guest's own configuration (`instances/instance2/config.json`).
    ///
    /// Held on the UI thread as well as by the worker because the video
    /// settings are consumed *here*, when painting the guest's screens, while
    /// audio and speed have to be forwarded as commands. Loaded when the guest
    /// starts so the window always edits the instance that is actually running.
    config: Config,
    show_emu_window: bool,
    show_audio_window: bool,
    show_video_window: bool,
}

impl Default for ThreadMode {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadMode {
    #[must_use]
    pub fn new() -> Self {
        ThreadMode {
            hub: None,
            worker: None,
            stop: Arc::new(AtomicBool::new(false)),
            frame: Arc::new(Mutex::new(GuestFrame::default())),
            input: Arc::new(Mutex::new(GuestInput::default())),
            commands: Arc::new(Mutex::new(Vec::new())),
            clock: Arc::new(AtomicU64::new(0)),
            last_error: None,
            is_open: false,
            textures: None,
            bottom_placement: None,
            assoc_trace: false,
            config: Config::default(),
            show_emu_window: false,
            show_audio_window: false,
            show_video_window: false,
        }
    }

    /// `true` while a guest is running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.worker.is_some()
    }

    /// The "Thread Mode" entry in the Multiplayer menu.
    pub fn menu_item(&mut self, ui: &mut egui::Ui) {
        if ui.checkbox(&mut self.is_open, "Thread Mode (2nd instance)").clicked() {
            ui.close();
        }
    }

    /// Boots a guest instance and links `host` to it over a shared
    /// [`LocalMpHub`].
    ///
    /// `host` is the main window's emulator; it joins the same hub as instance
    /// `0`. Both sides call `begin` immediately, because the Wi-Fi hardware
    /// only does so on a radio power-on edge that may already have happened.
    fn start(&mut self, host: &mut NDS, host_config: &Config) {
        if self.is_running() {
            return;
        }
        if host_config.last_rom_path.is_none() {
            self.last_error = Some("Load a ROM before starting Thread Mode".to_owned());
            return;
        }
        self.last_error = None;

        let hub = Arc::new(LocalMpHub::new());
        let mut host_transport = MpInterfaceTransport::new(LocalMp::from_hub(Arc::clone(&hub)), 0);
        host_transport.begin();
        host.set_mp_transport(Some(Box::new(host_transport)));

        // The guest boots now, while the host has been running for however
        // long the player took to get here. Wi-Fi frames carry a microsecond
        // timestamp and a receiver holds one back until its own clock reaches
        // it, so two consoles counting from their own boots read each other's
        // traffic as arriving from the future — or the distant past — and
        // never associate. Handing the newcomer its peer's reference puts both
        // on one timeline. See `NDS::wifi_clock_reference`.
        self.clock = Arc::new(AtomicU64::new(host.wifi_clock_reference()));

        self.stop = Arc::new(AtomicBool::new(false));
        *self.frame.lock().unwrap_or_else(|e| e.into_inner()) = GuestFrame::default();
        *self.input.lock().unwrap_or_else(|e| e.into_inner()) = GuestInput::default();
        self.commands.lock().unwrap_or_else(|e| e.into_inner()).clear();

        let stop = Arc::clone(&self.stop);
        let frame = Arc::clone(&self.frame);
        let input = Arc::clone(&self.input);
        let commands = Arc::clone(&self.commands);
        let hub_for_worker = Arc::clone(&hub);
        let clock = Arc::clone(&self.clock);

        // The guest runs from `instances/instance2/config.json`, not from a
        // copy of the host's: it owns its own saves, savestates, cheats, logs
        // and key bindings. Separate bindings are the point, not an accident --
        // two players sharing one keyboard need different keys.
        //
        // The ROM is the one exception. Local wireless play only means anything
        // if both instances run the same title, so the host's choice is
        // authoritative and is copied over whatever the guest's file records.
        let mut config = Config::load_for_instance(GUEST_INSTANCE);
        config.last_rom_path.clone_from(&host_config.last_rom_path);
        // The UI thread keeps its own copy: the video settings are consumed
        // when painting the guest's screens, which happens here, not on the
        // worker.
        self.config = config.clone();

        self.worker = Some(std::thread::spawn(move || {
            guest_main(&config, &hub_for_worker, &clock, &stop, &frame, &input, &commands);
            frame.lock().unwrap_or_else(|e| e.into_inner()).finished = true;
        }));
        self.hub = Some(hub);
    }

    /// Stops the guest and detaches the host from the hub, restoring the
    /// emulator to a state where the LAN room can install its own transport.
    fn stop_guest(&mut self, host: &mut NDS) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.hub = None;
        self.textures = None;
        self.bottom_placement = None;
        host.set_mp_transport(None);
    }

    /// Draws the control panel and, while a guest is running, its viewport.
    pub fn show(&mut self, ctx: &egui::Context, config: &Config, host: &mut NDS) {
        if self.is_running() {
            // Kept current so a guest that reboots (Reset, Import Save) rejoins
            // on the host's timeline rather than starting its clock over.
            self.clock.store(host.wifi_clock_reference(), Ordering::Relaxed);
        }
        self.control_panel(ctx, config, host);
        if self.is_running() {
            self.guest_viewport(ctx);
        }
    }

    fn control_panel(&mut self, ctx: &egui::Context, config: &Config, host: &mut NDS) {
        let mut open = self.is_open;
        egui::Window::new("Thread Mode").open(&mut open).resizable(true).show(ctx, |ui| {
            ui.label(
                "Runs a second DS in this process, on its own thread, in its own window, \
                 linked to the main instance by in-process channels. No sockets, no room, no \
                 packet loss -- if local play fails here, the fault is in the emulator rather \
                 than the network.",
            );
            ui.separator();

            ui.horizontal(|ui| {
                if self.is_running() {
                    if ui.button("Stop").clicked() {
                        self.stop_guest(host);
                    }
                    ui.label("guest running in its own window");
                } else if ui.button("Start guest instance").clicked() {
                    self.start(host, config);
                }
            });
            if let Some(err) = &self.last_error {
                ui.colored_label(egui::Color32::from_rgb(220, 120, 120), err);
            }

            if ui
                .checkbox(&mut self.assoc_trace, "Trace association responses (stderr)")
                .on_hover_text(
                    "Prints what each instance commits to its RX ring for an association                      response, then every W_RXBufDataRead the driver performs afterwards. Use                      it to tell 'the driver never read the frame' from 'it read the wrong                      place' from 'it read the right bytes and rejected them'.",
                )
                .changed()
            {
                nds_core::net::set_assoc_trace(self.assoc_trace);
            }

            // The trace also goes to stderr, which a GUI build started from
            // the desktop does not have. Showing it here is what makes it
            // usable during an actual reproduction.
            if self.assoc_trace {
                ui.horizontal(|ui| {
                    if ui.button("Clear trace").clicked() {
                        nds_core::net::clear_assoc_trace();
                    }
                    let lines = nds_core::net::assoc_trace_lines();
                    ui.weak(format!("{} lines", lines.len()));
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(lines.join("
"));
                    }
                });
                egui::ScrollArea::vertical().max_height(220.0).stick_to_bottom(true).show(
                    ui,
                    |ui| {
                        for line in nds_core::net::assoc_trace_lines() {
                            ui.label(egui::RichText::new(line).monospace().size(10.0));
                        }
                    },
                );
            }

            ui.separator();
            ui.strong("guest");
            let diag = self.frame.lock().unwrap_or_else(|e| e.into_inner()).diag.clone();
            ui.weak(if diag.is_empty() { "(not running)" } else { diag.as_str() });
            ui.add_space(4.0);
            ui.strong("host");
            ui.weak(diag_line(&host.wifi_diag_snapshot()));
            ui.weak(host.wifi_diag_snapshot().verdict(true));

            // The medium itself, which neither console can see. A host that
            // reports client failures while `stale` climbs is being answered
            // -- just always about the previous round, which means the two
            // wireless clocks have drifted apart rather than that the link is
            // down. Both counters flat means nobody is replying at all.
            if let Some(hub) = &self.hub {
                let status = hub.status();
                ui.add_space(4.0);
                ui.strong("airwaves");
                ui.weak(format!(
                    "connected=0x{:04X} host_inst={} replies collected={} stale={}",
                    status.connected_bitmask,
                    status.mp_host_inst,
                    status.collected_replies,
                    status.stale_replies,
                ));
            }
        });
        self.is_open = open;
    }

    /// The guest's own OS window: its two screens, its keyboard, its stylus.
    ///
    /// Uses an *immediate* viewport rather than a deferred one because the
    /// render closure has to borrow `self` (for the textures and the published
    /// frame); a deferred viewport's callback must be `Send + Sync + 'static`
    /// and could not.
    fn guest_viewport(&mut self, ctx: &egui::Context) {
        let viewport_id = egui::ViewportId::from_hash_of("lunaris_thread_mode_guest");
        let builder = egui::ViewportBuilder::default()
            .with_title("lunaris - Thread Mode guest (instance 1)")
            .with_inner_size([512.0, 768.0])
            // Every window this process opens needs the same opt-out the main
            // viewport takes (see `main`): winit's OS-level drag-and-drop
            // registration calls `OleInitialize`, which panics with
            // `RPC_E_CHANGED_MODE` when COM has already been initialised in
            // multithreaded mode elsewhere in the process. Leaving it enabled
            // here crashed the whole emulator the moment the guest window was
            // created.
            .with_drag_and_drop(false);

        let (screens, frames, finished, status, paused) = {
            let f = self.frame.lock().unwrap_or_else(|e| e.into_inner());
            (f.screens.clone(), f.frames, f.finished, f.status.clone(), f.paused)
        };

        let mut close_requested = false;
        ctx.show_viewport_immediate(viewport_id, builder, |ctx, _class| {
            if ctx.input(|i| i.viewport().close_requested()) {
                close_requested = true;
            }

            self.guest_menu_bar(ctx, paused);

            egui::TopBottomPanel::bottom("thread_mode_guest_status").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.weak(format!("frames {frames}"));
                    if paused {
                        ui.weak("paused");
                    }
                    if !status.is_empty() {
                        ui.separator();
                        ui.weak(&status);
                    }
                    if finished {
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 120, 120),
                            "guest thread exited",
                        );
                    }
                });
            });

            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(egui::Color32::BLACK)).show(
                ctx,
                |ui| {
                    if screens[0].is_empty() {
                        ui.centered_and_justified(|ui| ui.weak("booting guest instance..."));
                        return;
                    }
                    self.paint_screens(ui, &screens);
                },
            );

            self.settings_windows(ctx);
            self.collect_input(ctx);
        });

        if close_requested {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            self.hub = None;
            self.textures = None;
            self.bottom_placement = None;
        }

        // The guest emulates on its own thread, so without this the viewport
        // would only repaint on input and its screens would look frozen.
        ctx.request_repaint_after(Duration::from_millis(8));
    }

    /// Queues a menu action for the worker.
    fn send(&self, command: GuestCommand) {
        self.commands.lock().unwrap_or_else(|e| e.into_inner()).push(command);
    }

    /// The guest window's own menu bar.
    ///
    /// Mirrors the main window's File and Emulation menus. Every entry becomes
    /// a [`GuestCommand`] rather than acting directly, because the guest's
    /// `NDS` belongs to the worker thread. Savestate slots and the `.sav` are
    /// per-instance (`loader::state_dir_for_instance`,
    /// `loader::save_path_for_instance`), so the guest's "State 1" never
    /// overwrites the host's -- the two run independent timelines of one ROM.
    ///
    /// Deliberately absent: "Open ROM" (both instances must run the same ROM
    /// for local play to mean anything) and the Config/Video/Audio windows,
    /// which edit one shared `Config` the guest already reads.
    fn guest_menu_bar(&mut self, ctx: &egui::Context, paused: bool) {
        egui::TopBottomPanel::top("thread_mode_guest_menu").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    ui.menu_button("Save State", |ui| {
                        for slot in 1..=5 {
                            if ui.button(format!("State {slot}")).clicked() {
                                self.send(GuestCommand::SaveState(slot));
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Load State", |ui| {
                        for slot in 1..=5 {
                            if ui.button(format!("State {slot}")).clicked() {
                                self.send(GuestCommand::LoadState(slot));
                                ui.close();
                            }
                        }
                    });

                    if ui
                        .button("Import Save")
                        .on_hover_text("Copies the file over this instance's .sav and reboots it.")
                        .clicked()
                    {
                        // "dsv" accepts DeSmuME saves and "bin" covers raw
                        // flashcart dumps; both normalize to the same raw
                        // payload as a melonDS-style "sav". See
                        // `docs/design/ir-nand-foreign-sav-design.md` §3.3.
                        if let Some(file) = pollster::block_on(
                            rfd::AsyncFileDialog::new()
                                .add_filter("Save file", &["sav", "dsv", "bin"])
                                .pick_file(),
                        ) {
                            self.send(GuestCommand::ImportSave(file.path().to_path_buf()));
                        }
                        ui.close();
                    }
                    if ui.button("Export Save").clicked() {
                        if let Some(file) = pollster::block_on(
                            rfd::AsyncFileDialog::new()
                                .add_filter("Save file", &["sav"])
                                .save_file(),
                        ) {
                            self.send(GuestCommand::ExportSave(file.path().to_path_buf()));
                        }
                        ui.close();
                    }
                });

                ui.menu_button("Config", |ui| {
                    if ui.button("Emu Settings").clicked() {
                        self.show_emu_window = true;
                        ui.close();
                    }
                    if ui.button("Audio").clicked() {
                        self.show_audio_window = true;
                        ui.close();
                    }
                    if ui.button("Video").clicked() {
                        self.show_video_window = true;
                        ui.close();
                    }
                });

                ui.menu_button("Emulation", |ui| {
                    if ui.selectable_label(!paused, "Run").clicked() {
                        self.send(GuestCommand::SetPaused(false));
                        ui.close();
                    }
                    if ui.selectable_label(paused, "Stop").clicked() {
                        self.send(GuestCommand::SetPaused(true));
                        ui.close();
                    }
                    if ui.button("Reset").clicked() {
                        self.send(GuestCommand::Reset);
                        ui.close();
                    }
                });
            });
        });
    }

    /// The guest's Emu / Audio / Video windows.
    ///
    /// They edit **this instance's** configuration
    /// (`instances/instance2/config.json`), not the host's, and each change is
    /// persisted immediately as the main window does. Audio and speed reach the
    /// emulator as [`GuestCommand`]s because it lives on the worker thread;
    /// video settings need no round trip, since the screens are painted here.
    ///
    /// Input Settings is deliberately absent: the guest's key bindings live in
    /// its own `config.json`, and the capture UI is built around the main
    /// window's gamepad handle, which the guest does not share.
    fn settings_windows(&mut self, ctx: &egui::Context) {
        let mut open = self.show_emu_window;
        egui::Window::new("Guest: Emu Settings").open(&mut open).show(ctx, |ui| {
            let slider = egui::Slider::new(
                &mut self.config.emu_speed,
                lunaris_gui_common::config::MIN_EMU_SPEED
                    ..=lunaris_gui_common::config::MAX_EMU_SPEED,
            )
            .step_by(0.25)
            .suffix("x")
            .text("Speed");
            let mut changed = ui.add(slider).changed();
            if ui.button("Reset to 1.0x").clicked() {
                self.config.emu_speed = 1.0;
                changed = true;
            }
            if changed {
                self.send(GuestCommand::SetSpeed(self.config.emu_speed));
                self.config.save_for_instance(GUEST_INSTANCE);
            }
            ui.weak(
                "Running the guest off native speed desynchronises it from the host: the MP                  clock only tolerates the run-ahead the host's ack frames grant.",
            );
        });
        self.show_emu_window = open;

        let mut open = self.show_audio_window;
        egui::Window::new("Guest: Audio").open(&mut open).show(ctx, |ui| {
            let slider =
                egui::Slider::new(&mut self.config.audio_volume, 0.0..=100.0).text("Volume");
            if ui.add(slider).changed() {
                self.send(GuestCommand::SetVolume(self.config.audio_volume));
                self.config.save_for_instance(GUEST_INSTANCE);
            }
            ui.weak("Both instances output audio; set this to 0 to hear only the host.");
        });
        self.show_audio_window = open;

        let mut open = self.show_video_window;
        let mut changed = false;
        egui::Window::new("Guest: Video").open(&mut open).show(ctx, |ui| {
            egui::ComboBox::from_label("Layout")
                .selected_text(match self.config.video.screen_layout {
                    ScreenLayout::Vertical => "Vertical",
                    ScreenLayout::Horizontal => "Horizontal (Horizon)",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut self.config.video.screen_layout,
                            ScreenLayout::Vertical,
                            "Vertical",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut self.config.video.screen_layout,
                            ScreenLayout::Horizontal,
                            "Horizontal (Horizon)",
                        )
                        .changed();
                });
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.config.video.screen_gap, 0.0..=64.0)
                        .text("Screen gap"),
                )
                .changed();
            changed |=
                ui.checkbox(&mut self.config.video.integer_scaling, "Integer scaling").changed();
            let mut linear = self.config.video.filter == ScreenFilter::Linear;
            if ui.checkbox(&mut linear, "Linear filter").changed() {
                self.config.video.filter =
                    if linear { ScreenFilter::Linear } else { ScreenFilter::Nearest };
                changed = true;
            }
        });
        self.show_video_window = open;
        if changed {
            self.config.save_for_instance(GUEST_INSTANCE);
        }
    }

    /// Uploads the guest's screens and paints them using the same layout rules
    /// as the main window, so the two instances look and touch alike.
    fn paint_screens(&mut self, ui: &mut egui::Ui, screens: &[Vec<u16>; 2]) {
        // The *guest's* video settings, not the host's: each instance has its
        // own `config.json`.
        let video = self.config.video.clone();
        let avail = ui.available_size();
        let (top_rect, bottom_rect) = layout_screens(
            avail.x,
            avail.y,
            video.screen_layout,
            video.screen_gap,
            video.integer_scaling,
        );

        let size = [
            lunaris_gui_common::framebuffer::SCREEN_WIDTH,
            lunaris_gui_common::framebuffer::SCREEN_HEIGHT,
        ];
        let images = [
            egui::ColorImage::from_rgba_unmultiplied(size, &abgr1555_to_rgba8(&screens[0])),
            egui::ColorImage::from_rgba_unmultiplied(size, &abgr1555_to_rgba8(&screens[1])),
        ];
        // Honours the guest's own Video settings. Unscaled regardless: this
        // window exists to verify multiplayer, and an upscaler here would
        // compete with the host for GPU time on the same machine.
        let options = match video.filter {
            ScreenFilter::Nearest => egui::TextureOptions::NEAREST,
            ScreenFilter::Linear => egui::TextureOptions::LINEAR,
        };
        match &mut self.textures {
            Some([top, bottom]) => {
                top.set(images[0].clone(), options);
                bottom.set(images[1].clone(), options);
            }
            None => {
                let ctx = ui.ctx();
                self.textures = Some([
                    ctx.load_texture("thread_mode_top", images[0].clone(), options),
                    ctx.load_texture("thread_mode_bottom", images[1].clone(), options),
                ]);
            }
        }

        let response = ui.allocate_rect(ui.max_rect(), egui::Sense::click_and_drag());
        let origin = response.rect.min;
        if let Some([top, bottom]) = &self.textures {
            let paint = |rect: PlacementRect, tex: egui::TextureId| {
                ui.painter().image(
                    tex,
                    egui::Rect::from_min_size(
                        origin + egui::vec2(rect.x, rect.y),
                        egui::vec2(rect.width, rect.height),
                    ),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            };
            paint(top_rect, top.id());
            paint(bottom_rect, bottom.id());
        }

        self.bottom_placement = Some((origin, bottom_rect));
    }

    /// Reads this viewport's keyboard and pointer and publishes them for the
    /// worker.
    ///
    /// Reading the *held* key set rather than key events means a key released
    /// while the window was unfocused cannot stay stuck down; egui clears
    /// `keys_down` on focus loss.
    fn collect_input(&mut self, ctx: &egui::Context) {
        let (keys_down, pointer) = ctx.input(|i| {
            let pointer = if i.pointer.primary_down() { i.pointer.interact_pos() } else { None };
            (i.keys_down.clone(), pointer)
        });

        let keyboard: HashSet<KeyboardKey> =
            keys_down.into_iter().map(crate::input::egui_to_config_keyboard_key).collect();

        let touch = pointer.and_then(|pos| {
            let (origin, bottom) = self.bottom_placement?;
            point_to_touch_coords(pos.x - origin.x, pos.y - origin.y, bottom)
        });

        let mut slot = self.input.lock().unwrap_or_else(|e| e.into_inner());
        slot.keyboard = keyboard;
        slot.touch = touch;
    }
}

impl Drop for ThreadMode {
    fn drop(&mut self) {
        // The worker borrows nothing from the host, so it can be wound down
        // without one. Joining here keeps a closing window from leaving a
        // thread emulating into a `Mutex` nobody reads.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// One line of the MP diagnostic counters, in the same shape both instances
/// report so the two can be compared at a glance.
fn diag_line(d: &nds_core::nds::MpDiag) -> String {
    // `rx_accepted` far below what the peer transmitted says frames are being
    // lost, but not where. The drop counters are the other half of that
    // sentence, and omitting them made a real 88%-loss reading unactionable.
    let k = &d.drops;
    format!(
        "channel {} (ever {}, changes {}, on-channel {}% of {}ms) · tx dropped off-channel {} · \
         rf v{} idx {:?} now {:02X?} transfers {} \
         last id 0x{:02X} cmd {} · is_mp {} / client {} / aid {} · rx {} of {} polls (empty {}) \
         · loc {} beacon {} cmd {} reply {} · irq12 {}\nclassified: beacon {} mgmt {} cmd {} \
         ack {} reply {} · last assoc-resp: aid {} mac_good {} ts {}\ndelivered: irq0 {} \
         (masked {}) · driver reads: ring-RAM {} port {}\ndropped: rx_disabled {} powered_down {} \
         ring_unconfigured {} too_short {} bad_length {} channel_mismatch {} foreign_mp {} \
         filtered {} ring_full {} wep_off {}",
        d.channel,
        d.channel_ever,
        d.channel_changes,
        (d.channel_us * 100).checked_div(d.radio_us).unwrap_or(0),
        d.radio_us / 1000,
        d.tx_dropped_no_channel,
        d.rf_version,
        d.rf_channel_index,
        d.rf_regs_now,
        d.rf_transfers,
        d.rf_last_id,
        d.rf_last_cmd,
        d.is_mp,
        d.is_mp_client,
        d.aid,
        d.rx_accepted,
        d.rx_polls,
        d.rx_empty,
        d.loc_tx,
        d.beacon_tx,
        d.cmd_tx,
        d.reply_tx,
        d.irq12,
        d.rxflags_beacon,
        d.rxflags_mgmt,
        d.rxflags_cmd,
        d.rxflags_ack,
        d.rxflags_reply,
        d.last_assoc_aid,
        d.last_assoc_mac_good,
        d.last_assoc_timestamp,
        d.irq0_raised,
        d.irq0_masked,
        d.rx_ram_reads,
        d.rx_ring_reads,
        k.rx_disabled,
        k.rx_powered_down,
        k.ring_unconfigured,
        k.too_short,
        k.bad_length,
        k.channel_mismatch,
        k.foreign_mp,
        k.filtered,
        k.ring_full,
        k.wep_off,
    )
}

/// The guest thread's body: boot an instance, join the hub, then emulate at
/// roughly DS speed until asked to stop.
fn guest_main(
    config: &Config,
    hub: &Arc<LocalMpHub>,
    clock: &AtomicU64,
    stop: &AtomicBool,
    frame: &Mutex<GuestFrame>,
    input: &Mutex<GuestInput>,
    commands: &Mutex<Vec<GuestCommand>>,
) {
    let mut nds = boot_guest(config, hub, clock.load(Ordering::Relaxed));

    let mut next = Instant::now();
    let mut frames = 0u64;
    let mut paused = false;
    let mut speed = config.emu_speed.max(0.05);

    while !stop.load(Ordering::Relaxed) {
        let queued = std::mem::take(&mut *commands.lock().unwrap_or_else(|e| e.into_inner()));
        for command in queued {
            let status =
                apply_command(&mut nds, config, hub, clock, command, &mut paused, &mut speed);
            let mut slot = frame.lock().unwrap_or_else(|e| e.into_inner());
            slot.status = status;
            slot.paused = paused;
        }

        // Resolved once per emulated frame from whole state, exactly as the
        // main window does it, so both instances answer to the same bindings.
        let snapshot = input.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let state = InputState { keyboard: snapshot.keyboard, ..InputState::default() };
        crate::input::apply_input_bindings(&mut nds, &config.input_bindings, &state);
        match snapshot.touch {
            Some((x, y)) => nds.press_screen(x, y),
            None => nds.release_screen(),
        }

        if !paused {
            nds.emulate_frame();
        }
        frames += 1;

        // Publishing every frame would hold the lock 60 times a second for a
        // 192 KiB copy the UI samples far less often. Every other frame is
        // still above any display's refresh rate.
        if frames.is_multiple_of(2) {
            let [top, bottom] = nds.get_screens();
            let (top, bottom) = (top.clone(), bottom.clone());
            let diag = diag_line(&nds.wifi_diag_snapshot());
            let verdict = nds.wifi_diag_snapshot().verdict(true);
            let mut slot = frame.lock().unwrap_or_else(|e| e.into_inner());
            slot.screens = [top, bottom];
            slot.frames = frames;
            slot.diag = format!("{diag}\n{verdict}");
        }

        // Pace to real time. A guest that free-ran would race far ahead of the
        // host, and the MP sync clock only tolerates the run-ahead window the
        // host's ack frames actually grant. Recomputed each frame so a speed
        // change from the Emu Settings window takes effect immediately.
        next += Duration::from_secs_f64(1.0 / (NDS_FPS * f64::from(speed.max(0.05))));
        match next.checked_duration_since(Instant::now()) {
            Some(remaining) => std::thread::sleep(remaining),
            // Fell behind: give up the debt rather than sprint to catch up.
            None => next = Instant::now(),
        }
    }

    // The host flushes on exit; the guest has no window-close path of its own
    // that would otherwise do it, so an unflushed `.sav` would be lost.
    nds.flush_save();
}

/// Boots a guest instance and joins it to `hub`.
///
/// Used both at thread start and by [`GuestCommand::Reset`] /
/// [`GuestCommand::ImportSave`], which rebuild the emulator exactly as the main
/// window's Reset and Import Save do. Re-joining the hub matters: a fresh `NDS`
/// has no transport, so a reset instance would silently drop off the link.
fn boot_guest(config: &Config, hub: &Arc<LocalMpHub>, clock_epoch: u64) -> NDS {
    let mut nds = loader::load_rom_for_instance(config, GUEST_INSTANCE);
    let mut transport =
        MpInterfaceTransport::new(LocalMp::from_hub(Arc::clone(hub)), GUEST_INSTANCE);
    transport.begin();
    nds.set_mp_transport(Some(Box::new(transport)));
    // Before the cart has a chance to turn the radio on, which is when the
    // wireless clock takes its epoch.
    nds.set_wifi_clock_epoch(clock_epoch);
    nds
}

/// Applies one menu action, returning the line to show in the guest's status
/// bar.
fn apply_command(
    nds: &mut NDS,
    config: &Config,
    hub: &Arc<LocalMpHub>,
    clock: &AtomicU64,
    command: GuestCommand,
    paused: &mut bool,
    speed: &mut f32,
) -> String {
    let state_path = |slot: usize| {
        loader::state_dir_for_instance(config, GUEST_INSTANCE).map(|dir| {
            let _ = std::fs::create_dir_all(&dir);
            lunaris_gui_common::savestate::slot_path(&dir, slot)
        })
    };

    match command {
        GuestCommand::SaveState(slot) => match state_path(slot) {
            Some(path) => match lunaris_gui_common::savestate::save_to_file(nds, &path) {
                Ok(()) => format!("saved state {slot}"),
                Err(e) => format!("save state {slot} failed: {e}"),
            },
            None => "no ROM path configured".to_owned(),
        },
        GuestCommand::LoadState(slot) => match state_path(slot) {
            Some(path) => match lunaris_gui_common::savestate::load_from_file(nds, &path) {
                Ok(()) => {
                    *paused = false;
                    format!("loaded state {slot}")
                }
                Err(e) => format!("load state {slot} failed: {e}"),
            },
            None => "no ROM path configured".to_owned(),
        },
        GuestCommand::ImportSave(src) => {
            let Some(dst) = loader::save_path_for_instance(config, GUEST_INSTANCE) else {
                return "no save path configured".to_owned();
            };
            // Flush first: the reboot below reloads from disk, so anything
            // still buffered would resurface and overwrite the import.
            nds.flush_save();
            match std::fs::copy(&src, &dst) {
                Ok(_) => {
                    *nds = boot_guest(config, hub, clock.load(Ordering::Relaxed));
                    *paused = false;
                    format!("imported {}", src.display())
                }
                Err(e) => format!("import failed: {e}"),
            }
        }
        GuestCommand::ExportSave(dst) => match std::fs::write(&dst, nds.export_save()) {
            Ok(()) => format!("exported to {}", dst.display()),
            Err(e) => format!("export failed: {e}"),
        },
        GuestCommand::Reset => {
            nds.flush_save();
            *nds = boot_guest(config, hub, clock.load(Ordering::Relaxed));
            *paused = false;
            "reset".to_owned()
        }
        GuestCommand::SetPaused(value) => {
            *paused = value;
            if value { "paused".to_owned() } else { "running".to_owned() }
        }
        GuestCommand::SetVolume(volume) => {
            nds.set_audio_volume(volume);
            format!("volume {volume:.0}")
        }
        GuestCommand::SetSpeed(value) => {
            // Only native speed can stay locked to the audio clock; anything
            // else has to free-run or the device starves. Mirrors
            // `is_native_speed` in the main window.
            nds.set_audio_sync((value - 1.0).abs() < f32::EPSILON);
            *speed = value;
            format!("speed {value:.2}x")
        }
    }
}
