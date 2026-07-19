// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(feature = "release", windows_subsystem = "windows")]
#![expect(clippy::collapsible_if)]

mod cheat_editor;
mod config;
mod debug;
mod fonts;
mod input;
mod screens;
mod window;

use std::path::{Path, PathBuf};

use eframe::egui;
use lunaris_gui_common::framebuffer::{
    PlacementRect, ScreenLayout, layout_screens, point_to_touch_coords,
};
use nds_core::CheatMap;
use nds_core::nds::NDS;

use debug::{
    DebugWindow, MapsWindowState, PalettesWindowState, StatsWindow, TilesWindowState,
    VRAMWindowState,
};
use screens::ScreenTextures;

use crate::cheat_editor::CheatEditorState;

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

fn resolve_rom_path(config: &config::Config) -> Option<PathBuf> {
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

fn read_cheat_map(config: &config::Config) -> (CheatMap, String) {
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

fn create_nds(
    rom: &Path,
    config: &config::Config,
    cheat_editor_state: &mut CheatEditorState,
) -> NDS {
    let (cheat_map, cheat_txt) = read_cheat_map(config);
    cheat_editor_state.text_buffer = cheat_txt;

    let mut nds = NDS::load_rom(
        config.bios7_path.as_deref(),
        config.bios9_path.as_deref(),
        config.firmware_path.as_deref(),
        rom,
        config.audio_volume,
    );

    // Cheat Settings
    nds.set_cheat_map(cheat_map);
    nds.set_enable_cheats(config.enable_cheats);

    nds
}

fn save_state_to_slot(nds: &mut NDS, config: &config::Config, slot: usize) {
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
fn load_state_from_slot(nds: &mut NDS, config: &config::Config, slot: usize) -> bool {
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
    config: config::Config,
    paused: bool,
    screens: ScreenTextures,
    gilrs: gilrs::Gilrs,
    stylus_down: bool,
    cheat_editor_state: CheatEditorState,
    show_video_window: bool,
    show_audio_window: bool,
    debug: DebugState,
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
}

impl LunarisApp {
    fn new(
        ctx: &egui::Context,
        nds: NDS,
        config: config::Config,
        cheat_editor_state: CheatEditorState,
    ) -> Self {
        LunarisApp {
            nds,
            config,
            paused: false,
            screens: ScreenTextures::new(ctx),
            gilrs: gilrs::Gilrs::new().expect("failed to initialize gamepad backend"),
            stylus_down: false,
            cheat_editor_state,
            show_video_window: false,
            show_audio_window: false,
            debug: DebugState::new(),
            screens_dirty: true,
            emulated_frames: 0,
            last_title_update: std::time::Instant::now(),
            last_fps: 0.0,
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

        // loading a NDS file
        self.nds = create_nds(path, &self.config, &mut self.cheat_editor_state);

        // Save config
        self.config.last_rom_path = Some(path.clone());
        self.config.save();
        self.paused = false;
        self.screens_dirty = true;
    }

    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open ROM").clicked() {
                        // NOTE: If we don't run it async, `rfd` won't be displayed during the event.
                        let rom_dir = self.config.last_rom_path.as_ref().and_then(|p| p.parent());
                        let dialog = rfd::AsyncFileDialog::new().add_filter("NDS ROM", &["nds"]);

                        if let Some(p) = pollster::block_on(match rom_dir {
                            Some(dir) => dialog.set_directory(dir).pick_file(),
                            None => dialog.pick_file(),
                        }) {
                            let p = p.path().to_path_buf();

                            self.nds = create_nds(&p, &self.config, &mut self.cheat_editor_state);
                            self.config.last_rom_path = Some(p);
                            self.config.save();
                            self.paused = false;
                            self.screens_dirty = true;
                        }
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
                    });

                    if ui.button("Import Save").clicked() {
                        if let Some(p) =
                            rfd::FileDialog::new().add_filter("Save file", &["sav"]).pick_file()
                        {
                            match std::fs::read(&p) {
                                Ok(bytes) => self.nds.import_save(&bytes),
                                Err(err) => {
                                    eprintln!("Failed to read save file {}: {err}", p.display())
                                }
                            }
                        }
                        ui.close();
                    }

                    if ui.button("Export Save").clicked() {
                        if let Some(p) =
                            rfd::FileDialog::new().add_filter("Save file", &["sav"]).save_file()
                        {
                            if let Err(err) = std::fs::write(&p, self.nds.export_save()) {
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
                        if let Some(path) = self.config.last_rom_path.clone() {
                            self.nds =
                                create_nds(&path, &self.config, &mut self.cheat_editor_state);
                            self.paused = false;
                            self.screens_dirty = true;
                        }
                        ui.close();
                    }
                });

                ui.menu_button("Config", |ui| {
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
            });
        });
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
                    config::ScreenFilter::Nearest => "Nearest",
                    config::ScreenFilter::Linear => "Linear",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut self.config.video.filter,
                            config::ScreenFilter::Nearest,
                            "Nearest",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut self.config.video.filter,
                            config::ScreenFilter::Linear,
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
                    self.config.video.filter.texture_options(),
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

                self.handle_stylus(&response, origin, bottom_rect);
            },
        );
    }

    /// Maps pointer interaction on the bottom screen to NDS touch input.
    ///
    /// GBATEK "DS Touch Screen Controller (TSC)": the digitizer only covers
    /// the bottom LCD. See `docs/design/egui-migration-design.md` §8.5.
    fn handle_stylus(
        &mut self,
        response: &egui::Response,
        origin: egui::Pos2,
        bottom_rect: PlacementRect,
    ) {
        let pressed = response.is_pointer_button_down_on();
        if !pressed {
            if self.stylus_down {
                self.nds.release_screen();
                self.stylus_down = false;
            }
            return;
        }

        let Some(pos) = response.interact_pointer_pos() else {
            return;
        };
        let local = pos - origin;
        if let Some((x, y)) = point_to_touch_coords(local.x, local.y, bottom_rect) {
            self.nds.press_screen(x, y);
            self.stylus_down = true;
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

        if !self.paused {
            self.nds.emulate_frame();
            self.emulated_frames += 1;
            self.debug.stats.frame_completed();
            self.screens_dirty = true;
        }

        if !ctx.wants_keyboard_input() {
            input::apply_input(&mut self.nds, ctx, &mut self.gilrs);
            self.handle_state_hotkeys(ctx);
        }
        self.handle_file_drop(ctx);

        self.menu_bar(ctx);
        self.central_panel(ctx);
        self.debug.render(ctx, &mut self.nds);
        self.audio_window(ctx);
        self.video_window(ctx);
        self.update_title(ctx);

        if let Some(cheat_map) = self.cheat_editor_state.show_cheats(ctx, &self.config) {
            self.nds.set_cheat_map(cheat_map);
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
    nds_core::simplelog::TermLogger::init(
        nds_core::simplelog::LevelFilter::Off,
        nds_core::simplelog::Config::default(),
        nds_core::simplelog::TerminalMode::Mixed,
        nds_core::simplelog::ColorChoice::Auto,
    )
    .ok();

    let mut config = config::Config::load();
    let rom = resolve_rom_path(&config).expect("ROM required");
    config.last_rom_path = Some(rom.clone());

    let mut cheat_editor_state = CheatEditorState::default();
    let nds = create_nds(&rom, &config, &mut cheat_editor_state);

    let (icon_rgba, [icon_width, icon_height]) = icon();
    let viewport = egui::ViewportBuilder::default()
        .with_title("Lunaris(egui)")
        .with_icon(egui::IconData { rgba: icon_rgba, width: icon_width, height: icon_height })
        .with_inner_size([config.window.width, config.window.height])
        .with_position([config.window.pos_x, config.window.pos_y])
        // winit's OS-level drag-and-drop registration calls `OleInitialize`,
        // which panics with `RPC_E_CHANGED_MODE` if COM was already
        // initialized in multithreaded mode elsewhere in the process (seen
        // in this environment; root cause not yet isolated). Disabling it
        // avoids the crash; ROM loading still works via "Open ROM" / CLI arg
        // / remembered `last_rom_path`. `raw.dropped_files` drag-and-drop
        // from Explorer is a known regression versus the imgui front end
        // until the underlying COM conflict is root-caused.
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
