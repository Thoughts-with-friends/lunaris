// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(feature = "release", windows_subsystem = "windows")]
#![expect(clippy::collapsible_if)]

mod cheat_editor;
mod debug;
mod fonts;
mod input;
mod input_settings;
mod lan_room;
mod screens;
mod thread_mode;
mod window;

use std::path::{Path, PathBuf};

use debug::{
    DebugWindow, MapsWindowState, PalettesWindowState, StatsWindow, TilesWindowState,
    VRAMWindowState,
};
use eframe::egui;
use lunaris_gui_common::{
    config::{Config, ScreenFilter},
    framebuffer::{PlacementRect, ScreenLayout, layout_screens, point_to_touch_coords},
    input::stylus::StylusQueue,
    loader::create_save_path,
};
use nds_core::{CheatMap, nds::NDS};
use screens::ScreenTextures;

use crate::{cheat_editor::CheatEditorState, input_settings::InputSettingsState};

const fn texture_options(screen: ScreenFilter) -> egui::TextureOptions {
    match screen {
        ScreenFilter::Nearest => egui::TextureOptions::NEAREST,
        ScreenFilter::Linear => egui::TextureOptions::LINEAR,
    }
}

/// Groups every debug inspector window, mirroring the imgui front end's
/// `DebugState` struct in `gui/src/main.rs`.
struct DebugState {
    palettes: DebugWindow<PalettesWindowState>,
    maps: DebugWindow<MapsWindowState>,
    tiles: DebugWindow<TilesWindowState>,
    vram: DebugWindow<VRAMWindowState>,
    stats: StatsWindow,
}

impl DebugState {
    fn new() -> Self {
        DebugState {
            palettes: DebugWindow::new("Palettes"),
            maps: DebugWindow::new("Maps"),
            tiles: DebugWindow::new("Tiles"),
            vram: DebugWindow::new("VRAM"),
            stats: StatsWindow::new(),
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        self.palettes.menu_item(ui);
        self.maps.menu_item(ui);
        self.tiles.menu_item(ui);
        self.vram.menu_item(ui);
        self.stats.menu_item(ui);
    }

    fn render(&mut self, ctx: &egui::Context, nds: &mut NDS) {
        self.palettes.render(ctx, nds);
        self.maps.render(ctx, nds);
        self.tiles.render(ctx, nds);
        self.vram.render(ctx, nds);
        self.stats.render(ctx);
    }
}

/// Savestate hotkeys: F5-F9 save to slots 1-5, Shift+F5-F9 loads them.
const STATE_HOTKEYS: [(egui::Key, usize); 5] = [
    (egui::Key::F5, 1),
    (egui::Key::F6, 2),
    (egui::Key::F7, 3),
    (egui::Key::F8, 4),
    (egui::Key::F9, 5),
];

fn resolve_rom_path(config: &Config) -> Option<PathBuf> {
    if let Some(arg) = std::env::args().nth(1) {
        let p = PathBuf::from(arg);
        if p.exists() {
            return Some(p);
        }
    }

    if let Some(p) = &config.last_rom_path {
        if p.exists() {
            return Some(p.clone());
        }
    }

    rfd::FileDialog::new().add_filter("NDS ROM", &["nds"]).pick_file()
}

fn read_cheat_map(config: &Config) -> (CheatMap, String) {
    if let Some(rom_name) = config.last_rom_path.as_ref().and_then(|p| p.file_name()) {
        let cheat_dir = config.cheat_dir.as_path();
        let mut cheat_file = cheat_dir.join(rom_name);
        cheat_file.set_extension("txt");

        match std::fs::read_to_string(&cheat_file) {
            Ok(map_str) => {
                let map =
                    lunaris_gui_common::cheat_map::cheat_map_from_str(&map_str).unwrap_or_default();
                (map, map_str)
            }
            Err(err) => {
                nds_core::log::error!("{err}");
                let _ = std::fs::create_dir_all(cheat_dir);
                let _ = std::fs::write(cheat_file, "");
                (CheatMap::new(), String::new())
            }
        }
    } else {
        (CheatMap::new(), String::new())
    }
}

fn create_nds(config: &Config, cheat_editor_state: &mut CheatEditorState) -> NDS {
    let (cheat_map, cheat_txt) = read_cheat_map(config);
    cheat_editor_state.text_buffer = cheat_txt;

    let mut nds = lunaris_gui_common::loader::load_rom(config);

    // Cheat Settings
    nds.set_cheat_map(cheat_map);
    nds.set_enable_cheats(config.enable_cheats);

    // Re-applied on every (re)creation so a non-native speed survives Reset /
    // Open ROM / Import Save.
    nds.set_audio_sync(is_native_speed(config.emu_speed));

    nds
}

/// Whether `speed` is close enough to 1.0x to keep audio-clock pacing on.
fn is_native_speed(speed: f32) -> bool {
    (speed - 1.0).abs() < f32::EPSILON
}

fn save_state_to_slot(nds: &mut NDS, config: &Config, slot: usize) {
    if let Some(rom_path) = config.last_rom_path.as_ref().and_then(|p| p.file_stem()) {
        // ./states/<rom_name>/state_<n>.bin
        let state_dir = &config.save_state_dir.join(rom_path);
        let _ = std::fs::create_dir_all(state_dir);
        let path = lunaris_gui_common::savestate::slot_path(state_dir, slot);

        if let Err(e) = lunaris_gui_common::savestate::save_to_file(nds, &path) {
            nds_core::log::error!(target: "nds_core::savedata", "Failed to save state {slot}: {e}");
        }
    }
}

/// Loads slot `slot` into `nds`, returning `true` on success so the caller
/// can unpause emulation only when the load actually applied.
fn load_state_from_slot(nds: &mut NDS, config: &Config, slot: usize) -> bool {
    if let Some(rom_path) = config.last_rom_path.as_ref().and_then(|p| p.file_stem()) {
        let state_dir = &config.save_state_dir.join(rom_path);
        let _ = std::fs::create_dir_all(state_dir);
        let path = lunaris_gui_common::savestate::slot_path(state_dir, slot);

        match lunaris_gui_common::savestate::load_from_file(nds, &path) {
            Ok(()) => {
                nds_core::log::info!(target: "nds_core::savedata", "loaded state. {}", path.display());
                true
            }
            Err(e) => {
                nds_core::log::error!(target: "nds_core::savedata", "Failed to load state {slot}: {e}");
                false
            }
        }
    } else {
        false
    }
}

struct LunarisApp {
    nds: NDS,
    config: Config,
    paused: bool,
    screens: ScreenTextures,
    gilrs: gilrs::Gilrs,
    input_state: crate::input::InputState,
    /// Stylus states sampled from the host pointer, drained one per emulated
    /// frame by [`LunarisApp::emulate_batch`]. See [`Self::sample_stylus`].
    stylus: StylusQueue,
    /// Whether the pointer was pressing the emulated screen area as of the
    /// previous repaint. egui's own hit-testing decides this (so menus and
    /// floating windows keep their clicks), which is why it is one repaint
    /// old; only the press/release edges lag, never the position.
    stylus_gate: bool,
    /// Window-local origin and bottom-screen rectangle recorded by the
    /// previous [`Self::central_panel`], in egui points. Needed to map
    /// pointer positions before the panel of the current repaint is laid out.
    stylus_placement: Option<(egui::Pos2, PlacementRect)>,
    cheat_editor_state: CheatEditorState,
    input_settings: InputSettingsState,
    show_video_window: bool,
    show_audio_window: bool,
    show_emu_window: bool,
    /// Fractional frame budget for [`Config::emu_speed`], in NDS frames. See
    /// [`LunarisApp::emulate_batch`].
    frame_accum: f32,
    /// Wall-clock timestamp the current `frame_accum` was last advanced from.
    last_tick: std::time::Instant,
    debug: DebugState,
    lan_room: crate::lan_room::LanRoomState,
    thread_mode: crate::thread_mode::ThreadMode,
    /// Set whenever the displayed screens need to be re-converted/re-uploaded:
    /// a new frame was emulated, a ROM was (re)loaded, a savestate was
    /// loaded, or a video setting changed. Cleared every time
    /// [`Self::central_panel`] consumes it. See
    /// `docs/design/resolution-upscaling-design.md` §6.
    screens_dirty: bool,
    /// Frames actually emulated since the last title update. Deliberately
    /// counts emulated frames, not GUI repaints, so the displayed FPS drops
    /// to 0 if emulation stalls or is paused instead of masking a freeze.
    /// See `docs/design/egui-migration-design.md` §7.4.
    emulated_frames: u32,
    last_title_update: std::time::Instant,
    last_fps: f64,
    keyboard_keys: Vec<(lunaris_gui_common::input::enums::BindKey, egui::Key)>,
}

impl LunarisApp {
    /// Native NDS refresh rate (355 dots * 263 lines * 6 cycles, at 33.513982
    /// MHz). GBATEK "DS Video".
    const NDS_FPS: f32 = 59.8261;

    /// Longest wall-clock gap a single repaint is allowed to bill for.
    const MAX_DT: f32 = 0.25;

    /// Upper bound on frames emulated per repaint, so a slow host stays
    /// responsive to input and window events.
    const MAX_BATCH: u32 = 8;

    fn new(
        ctx: &egui::Context,
        nds: NDS,
        config: Config,
        cheat_editor_state: CheatEditorState,
    ) -> Self {
        let keyboard_keys = input::keyboard_keys(&config.input_bindings);

        LunarisApp {
            nds,
            config,
            paused: false,
            screens: ScreenTextures::new(ctx),
            gilrs: gilrs::Gilrs::new().expect("failed to initialize gamepad backend"),
            input_state: crate::input::InputState::default(),
            stylus: StylusQueue::default(),
            stylus_gate: false,
            stylus_placement: None,
            cheat_editor_state,
            input_settings: InputSettingsState::default(),
            show_video_window: false,
            show_audio_window: false,
            show_emu_window: false,
            frame_accum: 0.0,
            last_tick: std::time::Instant::now(),
            debug: DebugState::new(),
            lan_room: crate::lan_room::LanRoomState::default(),
            thread_mode: crate::thread_mode::ThreadMode::new(),
            screens_dirty: true,
            emulated_frames: 0,
            last_title_update: std::time::Instant::now(),
            last_fps: 0.0,
            keyboard_keys,
        }
    }

    fn update_window_info(&mut self, ctx: &egui::Context) {
        window::update_window_geometry(ctx, egui::ViewportId::ROOT, &mut self.config.window);
    }

    fn handle_state_hotkeys(&mut self, ctx: &egui::Context) {
        let (pressed, shift) = ctx.input(|i| {
            let pressed =
                STATE_HOTKEYS.iter().find(|(key, _)| i.key_pressed(*key)).map(|(_, s)| *s);
            (pressed, i.modifiers.shift)
        });
        let Some(slot) = pressed else { return };
        if shift {
            // A room member currently believed MP-ready would have its
            // timeline rewound out from under the other peers; refuse
            // rather than silently desync the room. See
            // `docs/design/design_lan.md` §13.3.
            if self.lan_room.blocks_state_load() {
                return;
            }
            if load_state_from_slot(&mut self.nds, &self.config, slot) {
                self.paused = false;
                self.screens_dirty = true;
            }
        } else {
            save_state_to_slot(&mut self.nds, &self.config, slot);
        }
    }

    fn handle_file_drop(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.len() != 1 {
            return;
        }
        let Some(path) = &dropped[0].path else { return };
        if path.extension().and_then(|e| e.to_str()) != Some("nds") {
            return;
        }
        self.config.last_rom_path = Some(path.clone());

        self.nds = create_nds(&self.config, &mut self.cheat_editor_state);
        self.lan_room.on_rom_changed(&self.nds);

        // Save config
        self.config.save();
        self.paused = false;
        self.screens_dirty = true;
    }

    fn open_rom(&mut self, save_path: Option<&Path>) {
        // NOTE: If we don't run it async, `rfd` won't be displayed during the event.
        let rom_dir = self.config.last_rom_path.as_ref().and_then(|p| p.parent());
        let dialog = rfd::AsyncFileDialog::new().add_filter("NDS ROM", &["nds"]);

        if let Some(p) = pollster::block_on(match rom_dir {
            Some(dir) => dialog.set_directory(dir).pick_file(),
            None => dialog.pick_file(),
        }) {
            let p = p.path().to_path_buf();

            if let Some(save_path) = save_path {
                let dst = create_save_path(&self.config).unwrap();
                let _ = std::fs::copy(save_path, &dst);
            }
            self.config.last_rom_path = Some(p);
            self.nds = create_nds(&self.config, &mut self.cheat_editor_state);
            self.lan_room.on_rom_changed(&self.nds);
            self.config.save();
            self.paused = false;
            self.screens_dirty = true;
        }
    }

    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open ROM").clicked() {
                        self.open_rom(None);
                        ui.close();
                    }

                    ui.menu_button("Save State", |ui| {
                        for slot in 1..=5 {
                            if ui.button(format!("State {slot}")).clicked() {
                                save_state_to_slot(&mut self.nds, &self.config, slot);
                                ui.close();
                            }
                        }
                    });

                    let blocks_state_load = self.lan_room.blocks_state_load();
                    ui.add_enabled_ui(!blocks_state_load, |ui| {
                        ui.menu_button("Load State", |ui| {
                            for slot in 1..=5 {
                                if ui.button(format!("State {slot}")).clicked() {
                                    if load_state_from_slot(&mut self.nds, &self.config, slot) {
                                        self.paused = false;
                                        self.screens_dirty = true;
                                    }
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_disabled_hover_text(
                            "Disabled while this instance is MP-ready in a LAN room: loading a \
                             savestate would rewind this instance's timeline out from under the \
                             other room members.",
                        );
                    });

                    if ui
                        .button("Import Save")
                        .on_hover_text("Loading the save file and restarting the emulator.")
                        .clicked()
                    {
                        // "dsv" accepts DeSmuME saves (footer stripped by
                        // NDS::import_save) and "bin" covers raw flashcart
                        // dumps; both normalize to the same raw payload as
                        // a melonDS-style "sav". See
                        // `docs/design/ir-nand-foreign-sav-design.md` §3.3.
                        if let Some(p) = pollster::block_on(
                            rfd::AsyncFileDialog::new()
                                .add_filter("Save file", &["sav", "dsv", "bin"])
                                .pick_file(),
                        ) {
                            let save_path = p.path();
                            if let Some(dst) = create_save_path(&self.config) {
                                let _ = std::fs::copy(save_path, &dst);
                                self.nds = create_nds(&self.config, &mut self.cheat_editor_state);
                                self.lan_room.on_rom_changed(&self.nds);
                            } else {
                                self.open_rom(Some(save_path));
                            };
                        }
                        ui.close();
                    }

                    if ui.button("Export Save").clicked() {
                        if let Some(p) = pollster::block_on(
                            rfd::AsyncFileDialog::new()
                                .add_filter("Save file", &["sav"])
                                .save_file(),
                        ) {
                            let p = p.path();
                            if let Err(err) = std::fs::write(p, self.nds.export_save()) {
                                eprintln!("Failed to write save file {}: {err}", p.display());
                            }
                        }
                        ui.close();
                    }

                    if ui.button("Exit").clicked() {
                        self.nds.flush_save();
                        std::process::exit(0);
                    }
                });

                ui.menu_button("Emulation", |ui| {
                    if ui.selectable_label(!self.paused, "Run").clicked() {
                        self.paused = false;
                        ui.close();
                    }
                    if ui.selectable_label(self.paused, "Stop").clicked() {
                        self.paused = true;
                        ui.close();
                    }
                    if ui.button("Reset").clicked() {
                        self.nds = create_nds(&self.config, &mut self.cheat_editor_state);
                        self.lan_room.on_rom_changed(&self.nds);
                        self.paused = false;
                        self.screens_dirty = true;
                        ui.close();
                    }
                });

                ui.menu_button("Config", |ui| {
                    if ui.button("Emu Settings").clicked() {
                        self.show_emu_window = true;
                        ui.close();
                    }
                    // The input settings window
                    self.input_settings.menu_item(ui, &self.config);

                    if ui.button("Audio").clicked() {
                        self.show_audio_window = true;
                        ui.close();
                    }
                    if ui.button("Video").clicked() {
                        self.show_video_window = true;
                        ui.close();
                    }
                });

                ui.menu_button("Tools", |ui| {
                    if ui.checkbox(&mut self.config.enable_cheats, "Enable Cheats").clicked() {
                        self.nds.set_enable_cheats(self.config.enable_cheats);
                    }

                    if ui.button("Cheat Codes").clicked() {
                        self.cheat_editor_state.is_open = !self.cheat_editor_state.is_open;
                        ui.close();
                    }
                });

                ui.menu_button("Debug", |ui| {
                    self.debug.menu(ui);
                });

                ui.menu_button("Multiplayer", |ui| {
                    self.lan_room.menu_item(ui);
                    self.thread_mode.menu_item(ui);
                });
            });
        });
    }

    /// Emulates however many NDS frames the elapsed wall-clock time is worth
    /// at the configured speed multiplier.
    ///
    /// Pacing is anchored to real time rather than to the repaint rate so that
    /// "1.0x" means native NDS speed on any display. (Previously one frame was
    /// emulated per repaint, which ran the emulator at ~1.75x on a high-refresh
    /// monitor.)
    fn emulate_batch(&mut self) {
        // A large `dt` — first frame, window drag, breakpoint — must not turn
        // into a burst of catch-up frames.
        let dt = self.last_tick.elapsed().as_secs_f32().min(Self::MAX_DT);
        self.last_tick = std::time::Instant::now();

        // A room member has to stay on the timeline its peers assume. The
        // slider is also disabled in-room, but that alone wouldn't stop
        // someone setting 4x *before* joining.
        let speed = if self.lan_room.blocks_state_load() { 1.0 } else { self.config.emu_speed };

        self.frame_accum += dt * Self::NDS_FPS * speed;

        let emulated = (self.frame_accum.max(0.0) as u32).min(Self::MAX_BATCH);
        for frame in 0..emulated {
            self.frame_accum -= 1.0;

            // Applied per frame, not per repaint: a batch that has fallen
            // behind still walks the stylus along the sampled path instead of
            // running every one of its frames on the same stale position.
            if let Some(sample) = self.stylus.next_sample((emulated - frame) as usize) {
                match sample {
                    Some((x, y)) => self.nds.press_screen(x, y),
                    None => self.nds.release_screen(),
                }
            }

            self.nds.emulate_frame();
            self.emulated_frames += 1;
            self.debug.stats.frame_completed();
        }

        // Only the last frame of a batch is ever displayed, so the texture
        // upload happens once per repaint rather than once per emulated frame.
        if emulated > 0 {
            self.screens_dirty = true;
        }

        // Budget left over means the host couldn't keep up. Drop it instead of
        // accruing unpayable debt, which would otherwise pin every later
        // repaint to `MAX_BATCH` and inflate input latency.
        if self.frame_accum > 1.0 {
            self.frame_accum = 0.0;
        }
    }

    /// Emulation speed control. See `Config::emu_speed`.
    fn emu_window(&mut self, ctx: &egui::Context) {
        if !self.show_emu_window {
            return;
        }
        let mut open = self.show_emu_window;
        egui::Window::new("Emu Settings").open(&mut open).default_size([320.0, 110.0]).show(
            ctx,
            |ui| {
                // Altering the speed desynchronizes a LAN room for the same
                // reason a savestate load does: the peers assume a shared
                // ~60fps timeline (`LanConfig::runahead_us`).
                let blocks = self.lan_room.blocks_state_load();
                ui.add_enabled_ui(!blocks, |ui| {
                    let slider = egui::Slider::new(
                        &mut self.config.emu_speed,
                        lunaris_gui_common::config::MIN_EMU_SPEED
                            ..=lunaris_gui_common::config::MAX_EMU_SPEED,
                    )
                    .step_by(0.25)
                    .suffix("x")
                    .text("Speed");

                    if ui
                        .add(slider)
                        .on_disabled_hover_text(
                            "Disabled while this instance is MP-ready in a LAN room: running off \
                             native speed would desync the other room members.",
                        )
                        .changed()
                    {
                        self.nds.set_audio_sync(is_native_speed(self.config.emu_speed));
                        self.config.save();
                    }

                    if ui.button("Reset to 1.0x").clicked() {
                        self.config.emu_speed = 1.0;
                        self.nds.set_audio_sync(true);
                        self.config.save();
                    }
                });

                if !is_native_speed(self.config.emu_speed) {
                    ui.weak(
                        "Audio-clock pacing is off at non-native speed, so sound is choppy. The \
                         speed actually reached is bounded by how many frames this host can \
                         emulate per second.",
                    );
                }
            },
        );
        self.show_video_window_close_guard(open, |s| &mut s.show_emu_window);
    }

    /// Button remapping. See [`crate::input_settings`].
    fn input_settings_window(&mut self, ctx: &egui::Context) {
        if self.input_settings.show(ctx, &mut self.config)
            != crate::input_settings::InputSettingsAction::Applied
        {
            return;
        }

        // Both caches are derived from the old bindings: `keyboard_keys` is
        // the list of egui keys polled every frame (a new binding does
        // nothing until it is rebuilt), and `input_state` can still hold a key
        // that the new bindings no longer mention, which would leave that NDS
        // button stuck down.
        self.keyboard_keys = input::keyboard_keys(&self.config.input_bindings);
        self.input_state = crate::input::InputState::default();
        self.config.save();
    }

    fn audio_window(&mut self, ctx: &egui::Context) {
        if !self.show_audio_window {
            return;
        }
        let mut open = self.show_audio_window;
        egui::Window::new("Audio").open(&mut open).default_size([300.0, 80.0]).show(ctx, |ui| {
            let slider =
                egui::Slider::new(&mut self.config.audio_volume, 0.0..=100.0).text("Volume");
            if ui.add(slider).changed() {
                self.nds.set_audio_volume(self.config.audio_volume);
                self.config.save();
            }
        });
        self.show_video_window_close_guard(open, |s| &mut s.show_audio_window);
    }

    fn video_window(&mut self, ctx: &egui::Context) {
        if !self.show_video_window {
            return;
        }
        let mut open = self.show_video_window;
        let mut changed = false;
        egui::Window::new("Video").open(&mut open).default_size([260.0, 180.0]).show(ctx, |ui| {
            egui::ComboBox::from_label("Filter")
                .selected_text(match self.config.video.filter {
                    ScreenFilter::Nearest => "Nearest",
                    ScreenFilter::Linear => "Linear",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut self.config.video.filter,
                            ScreenFilter::Nearest,
                            "Nearest",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut self.config.video.filter,
                            ScreenFilter::Linear,
                            "Linear",
                        )
                        .changed();
                });

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
            changed |=
                ui.checkbox(&mut self.config.video.show_fps_overlay, "Show FPS overlay").changed();

            ui.separator();

            egui::ComboBox::from_label("Upscaler")
                .selected_text(match self.config.video.upscale_method {
                    lunaris_gui_common::upscale::UpscaleMethod::None => "None",
                    lunaris_gui_common::upscale::UpscaleMethod::Xbrz => "xBRZ",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut self.config.video.upscale_method,
                            lunaris_gui_common::upscale::UpscaleMethod::None,
                            "None",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut self.config.video.upscale_method,
                            lunaris_gui_common::upscale::UpscaleMethod::Xbrz,
                            "xBRZ",
                        )
                        .changed();
                });

            let upscaler_active = self.config.video.upscale_method
                != lunaris_gui_common::upscale::UpscaleMethod::None;
            ui.add_enabled_ui(upscaler_active, |ui| {
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut self.config.video.upscale_factor,
                            lunaris_gui_common::upscale::MIN_FACTOR
                                ..=lunaris_gui_common::upscale::MAX_FACTOR,
                        )
                        .text("Scale factor"),
                    )
                    .changed();
            });

            if upscaler_active {
                // The actually-computed factor is capped to what the
                // on-screen size can show (see `screens::effective_factor`),
                // which is usually well below the nominal slider value —
                // show the real number so this isn't confusing.
                let effective = self.screens.last_effective_factor() as usize;
                let nominal = self.config.video.upscale_factor as usize;
                ui.weak(format!(
                    "Output: {}x{} per screen{}",
                    lunaris_gui_common::framebuffer::SCREEN_WIDTH * effective,
                    lunaris_gui_common::framebuffer::SCREEN_HEIGHT * effective,
                    if effective < nominal {
                        format!(" (effective {effective}x, display-limited)")
                    } else {
                        String::new()
                    },
                ));
            }
        });
        if changed {
            self.screens_dirty = true;
            self.config.save();
        }
        self.show_video_window_close_guard(open, |s| &mut s.show_video_window);
    }

    /// Small helper so closing the window's [x] button and toggling from the
    /// menu both write back to the right flag without borrow conflicts.
    fn show_video_window_close_guard(
        &mut self,
        open: bool,
        field: impl Fn(&mut Self) -> &mut bool,
    ) {
        *field(self) = open;
    }

    fn central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(egui::Color32::BLACK)).show(
            ctx,
            |ui| {
                // Layout is computed before the texture update because the
                // resulting on-screen rect size caps how large an upscale
                // factor is actually worth computing/uploading — see
                // `ScreenTextures::update` and `screens::effective_factor`.
                let avail = ui.available_size();
                let (top_rect, bottom_rect) = layout_screens(
                    avail.x,
                    avail.y,
                    self.config.video.screen_layout,
                    self.config.video.screen_gap,
                    self.config.video.integer_scaling,
                );
                let display_px = top_rect.width * ctx.pixels_per_point();

                let dirty = std::mem::take(&mut self.screens_dirty);
                self.screens.update(
                    ctx,
                    &self.nds,
                    texture_options(self.config.video.filter),
                    self.config.video.upscale_method,
                    self.config.video.upscale_factor,
                    display_px,
                    dirty,
                );

                let response = ui.allocate_rect(ui.max_rect(), egui::Sense::click_and_drag());
                let origin = response.rect.min;

                let paint_screen = |ui: &egui::Ui, rect: PlacementRect, tex: egui::TextureId| {
                    let screen_rect = egui::Rect::from_min_size(
                        origin + egui::vec2(rect.x, rect.y),
                        egui::vec2(rect.width, rect.height),
                    );
                    ui.painter().image(
                        tex,
                        screen_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                };
                paint_screen(ui, top_rect, self.screens.top.id());
                paint_screen(ui, bottom_rect, self.screens.bottom.id());

                if self.config.video.show_fps_overlay {
                    ui.painter().text(
                        origin + egui::vec2(4.0, 4.0),
                        egui::Align2::LEFT_TOP,
                        format!("{:.1} FPS", self.last_fps),
                        egui::FontId::monospace(14.0),
                        egui::Color32::from_rgb(0, 255, 0),
                    );
                }

                self.record_stylus_placement(&response, origin, bottom_rect);
            },
        );
    }

    /// Records where the emulated screens ended up and whether the pointer is
    /// pressing them, for the next repaint's [`Self::sample_stylus`].
    ///
    /// The panel is only laid out here, after emulation has already run for
    /// this repaint, so nothing is applied to the emulator from this point —
    /// doing so is what used to cost a full repaint of stylus latency.
    ///
    /// GBATEK "DS Touch Screen Controller (TSC)": the digitizer only covers
    /// the bottom LCD. See `docs/design/egui-migration-design.md` §8.5.
    fn record_stylus_placement(
        &mut self,
        response: &egui::Response,
        origin: egui::Pos2,
        bottom_rect: PlacementRect,
    ) {
        self.stylus_gate = response.is_pointer_button_down_on();
        self.stylus_placement = Some((origin, bottom_rect));

        // egui clears `is_pointer_button_down_on` as soon as it sees the
        // release event, so a tap whose press and release both land inside a
        // single repaint reports "not pressed" throughout and would never
        // reach the emulator. `clicked()` reports exactly that case, and the
        // tap is already over, so both of its edges are queued here rather
        // than inferred from pointer state a repaint later.
        if response.clicked() {
            let touch = response.interact_pointer_pos().and_then(|pos| {
                let local = pos - origin;
                point_to_touch_coords(local.x, local.y, bottom_rect)
            });
            if touch.is_some() {
                self.stylus.push(touch);
                self.stylus.push(None);
            }
        }
    }

    /// Queues this repaint's stylus states, before any frame is emulated.
    ///
    /// Every pointer motion egui received since the last repaint is turned
    /// into a sample, so a fast drag reaches the emulator as the path the
    /// pointer actually took rather than as a single endpoint one repaint
    /// late. [`StylusQueue`] decides how many of them a given batch consumes.
    fn sample_stylus(&mut self, ctx: &egui::Context) {
        let Some((origin, bottom_rect)) = self.stylus_placement else { return };

        // Only pointer positions are read here; whether the press counts as a
        // stylus press is `stylus_gate`, i.e. egui's own hit-testing from the
        // previous repaint. That keeps clicks on the menu bar and on floating
        // windows out of the emulator without re-implementing hit-testing.
        if !self.stylus_gate {
            self.stylus.push(None);
            return;
        }

        let to_touch = |pos: egui::Pos2| {
            let local = pos - origin;
            point_to_touch_coords(local.x, local.y, bottom_rect)
        };

        let samples = ctx.input(|i| {
            let mut samples = Vec::new();

            // The gate means the pointer was pressing the screen as of the
            // previous repaint, so the pen starts this repaint down; the
            // button events below are what lift it. Tracking this matters for
            // a release followed by more motion inside one repaint, which
            // must not be queued as a continued press.
            let mut down = true;

            for event in &i.raw.events {
                match event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        ..
                    } => {
                        down = *pressed;
                        samples.push(if down { to_touch(*pos) } else { None });
                    }
                    // Dragging off the digitizer is a release on hardware;
                    // leaving the press latched instead would keep the NDS
                    // holding a stale touch, so no later tap produces a press
                    // edge. `to_touch` returns `None` off-rect, which is that
                    // release.
                    egui::Event::PointerMoved(pos) => {
                        samples.push(if down { to_touch(*pos) } else { None });
                    }
                    egui::Event::PointerGone => {
                        down = false;
                        samples.push(None);
                    }
                    _ => {}
                }
            }

            // A held, motionless pointer emits no events at all; re-sampling
            // its last position keeps a press that began on an earlier
            // repaint alive. `StylusQueue::push` coalesces the repeats.
            if samples.is_empty() {
                samples.push(match i.pointer.primary_down() {
                    true => i.pointer.latest_pos().and_then(to_touch),
                    false => None,
                });
            }
            samples
        });

        for sample in samples {
            self.stylus.push(sample);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let elapsed = self.last_title_update.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            self.last_fps = self.emulated_frames as f64 / elapsed;
            self.emulated_frames = 0;
            self.last_title_update = std::time::Instant::now();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                "Lunaris(egui) - {:.2} FPS",
                self.last_fps
            )));
        }
    }
}

impl eframe::App for LunarisApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        self.update_window_info(ctx);

        if self.paused {
            // Don't let a pause bank up a burst of frames — or a backlog of
            // stylus samples nothing is draining — on resume.
            self.frame_accum = 0.0;
            self.stylus.clear();
            self.last_tick = std::time::Instant::now();
        } else {
            // Sampled before emulating, so the frames emulated below act on
            // this repaint's pointer position rather than the previous one's.
            self.sample_stylus(ctx);
            self.emulate_batch();
        }

        self.input_settings.poll_capture(ctx, &mut self.gilrs);

        if !ctx.wants_keyboard_input() && !self.input_settings.is_capturing() {
            ctx.input(|i| {
                for (_, egui_key) in &self.keyboard_keys {
                    let egui_key = *egui_key;
                    input::update_keyboard_input(
                        &mut self.input_state,
                        egui_key,
                        i.key_down(egui_key),
                    );
                }
            });

            // detect next pressed key
            while self.gilrs.next_event().is_some() {}

            input::update_gamepad_input(
                &self.gilrs,
                &mut self.input_state,
                self.config.joystick_id,
            );
            input::apply_input_bindings(
                &mut self.nds,
                &self.config.input_bindings,
                &self.input_state,
            );
            self.handle_state_hotkeys(ctx);
        }

        self.handle_file_drop(ctx);

        self.menu_bar(ctx);
        self.central_panel(ctx);
        self.debug.render(ctx, &mut self.nds);
        self.thread_mode.show(ctx, &self.config, &mut self.nds);
        self.emu_window(ctx);
        self.audio_window(ctx);
        self.video_window(ctx);
        self.input_settings_window(ctx);
        self.update_title(ctx);

        if let Some(cheat_map) = self.cheat_editor_state.show_cheats(ctx, &self.config) {
            self.nds.set_cheat_map(cheat_map);
        }

        if matches!(
            self.lan_room.show(ctx, &mut self.config, &mut self.nds),
            crate::lan_room::LanUiAction::SaveConfig
        ) {
            self.config.save();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Flushes any cartridge save-chip writes that never released
        // chip-select before the window closed. See
        // `docs/design/sav-backup-redesign.md` §4.1.
        self.nds.flush_save();
        self.config.save();
    }
}

fn main() -> eframe::Result<()> {
    #[cfg(not(feature = "release"))]
    let _ = lunaris_gui_common::log::setup_logging();

    let mut config = lunaris_gui_common::config::Config::load();
    let rom = resolve_rom_path(&config).expect("ROM required");
    config.last_rom_path = Some(rom);

    let mut cheat_editor_state = CheatEditorState::default();
    let nds = create_nds(&config, &mut cheat_editor_state);

    let (icon_rgba, [icon_width, icon_height]) = icon();
    let viewport = egui::ViewportBuilder::default()
        .with_title("Lunaris(egui)")
        .with_icon(egui::IconData { rgba: icon_rgba, width: icon_width, height: icon_height })
        .with_inner_size([config.window.width, config.window.height])
        .with_position([config.window.pos_x, config.window.pos_y])
        // winit's OS-level drag-and-drop registration calls `OleInitialize`,
        // which requires the thread to be in a single-threaded COM apartment
        // and panics with `RPC_E_CHANGED_MODE` if COM was already initialized
        // as multithreaded.
        //
        // Root cause: `create_nds` above runs on this thread *before*
        // `run_native`, and building an `NDS` constructs the SPU's audio
        // output, whose `cpal::default_host()` selects WASAPI and initializes
        // COM multithreaded (`core/src/hw/spu/audio.rs`). By the time winit
        // creates the window, the apartment is already set.
        //
        // Disabling drag-and-drop avoids the crash; ROM loading still works
        // via "Open ROM" / CLI arg / remembered `last_rom_path`.
        // `raw.dropped_files` drag-and-drop from Explorer stays a known
        // regression versus the imgui front end. Restoring it means keeping
        // COM untouched on this thread until the window exists -- i.e.
        // constructing the `NDS` inside the `run_native` creation closure, or
        // opening the audio device off-thread.
        //
        // **Every other viewport this process opens needs the same opt-out**;
        // see `crate::thread_mode`, whose guest window crashed the emulator
        // until it did.
        .with_drag_and_drop(false);

    let options = eframe::NativeOptions { viewport, ..Default::default() };

    eframe::run_native(
        "Lunaris(egui)",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            fonts::setup_custom_fonts::<&str>(&cc.egui_ctx, None);
            Ok(Box::new(LunarisApp::new(&cc.egui_ctx, nds, config, cheat_editor_state)))
        }),
    )
}

/// Get icon
///
/// (rgba_data, [width, height])
///
/// # Panics
/// Not icon load
#[inline]
fn icon() -> (Vec<u8>, [u32; 2]) {
    ico_to_rgba(include_bytes!("../../../docs/icons/icon.ico"))
}

#[expect(clippy::unwrap_used)]
fn ico_to_rgba(bytes: &[u8]) -> (Vec<u8>, [u32; 2]) {
    let cursor = std::io::Cursor::new(bytes);
    let ico = ico::IconDir::read(cursor).unwrap();
    let entry = ico.entries().first().unwrap();
    let image = entry.decode().unwrap();
    let width = image.width();
    let height = image.height();
    (image.rgba_data().to_vec(), [width, height])
}
