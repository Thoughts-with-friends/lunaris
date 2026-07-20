// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(feature = "release", windows_subsystem = "windows")]
#![expect(clippy::collapsible_if)]

mod debug;
mod display;
mod input;
mod savestate;

use std::{borrow::Borrow as _, collections::HashSet, fs::File, path::PathBuf};

use glfw::Key;
use imgui::{Condition, MenuItem, Slider, Ui, Window, im_str};

use lunaris_gui_common::loader::load_rom;
use nds_core::{
    nds::NDS,
    simplelog::{
        ColorChoice, CombinedLogger, Config, ConfigBuilder, LevelFilter, SharedLogger, TermLogger,
        TerminalMode, WriteLogger,
    },
};

use self::{
    debug::{
        DebugWindow, MapsWindowState, PalettesWindowState, StatsWindow, TilesWindowState,
        VRAMWindowState,
    },
    display::{Display, StateAction},
};

// =========================================================
// Logging
// =========================================================

fn setup_logging() {
    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![TermLogger::new(
        LevelFilter::Off,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )];

    let arm7 = File::create("logs/arm7.log");
    let arm9 = File::create("logs/arm9.log");
    let save = File::create("logs/savedata.log");

    let mut config = ConfigBuilder::new();
    config
        .set_time_level(LevelFilter::Off)
        .set_thread_level(LevelFilter::Off)
        .set_target_level(LevelFilter::Off)
        .set_location_level(LevelFilter::Off)
        .set_time_level(LevelFilter::Off)
        .set_max_level(LevelFilter::Off);

    if let Ok(file) = arm7 {
        loggers.push(WriteLogger::new(
            LevelFilter::Off,
            config.clone().add_filter_allow_str("nds_core::arm7").build(),
            file,
        ));
    }

    if let Ok(file) = arm9 {
        loggers.push(WriteLogger::new(
            LevelFilter::Off,
            config.clone().add_filter_allow_str("nds_core::arm9").build(),
            file,
        ));
    }

    if let Ok(file) = save {
        loggers.push(WriteLogger::new(
            LevelFilter::Off,
            config.clone().add_filter_allow_str("nds_core::savedata").build(),
            file,
        ));
    }

    CombinedLogger::init(loggers).unwrap();
}

// =========================================================
// ROM
// =========================================================

fn resolve_rom_path(config: &lunaris_gui_common::config::Config) -> Option<PathBuf> {
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

fn create_nds(config: &lunaris_gui_common::config::Config) -> NDS {
    load_rom(config)
}

// =========================================================
// Save states
// =========================================================

fn state_slot_path(config: &lunaris_gui_common::config::Config, slot: usize) -> PathBuf {
    config.save_state_dir.join(format!("state_{slot}.bin"))
}

fn save_state_to_slot(nds: &mut NDS, config: &lunaris_gui_common::config::Config, slot: usize) {
    let path = state_slot_path(config, slot);
    if let Err(e) = savestate::save_to_file(nds, &path) {
        nds_core::log::error!(target: "nds_core::savedata", "Failed to save state {slot}: {e}");
    }
}

/// Loads slot `slot` into `nds`. Returns `true` on success, so callers can
/// unpause emulation only when the load actually applied.
fn load_state_from_slot(
    nds: &mut NDS,
    config: &lunaris_gui_common::config::Config,
    slot: usize,
) -> bool {
    let path = state_slot_path(config, slot);
    match savestate::load_from_file(nds, &path) {
        Ok(()) => {
            nds_core::log::info!(target: "nds_core::savedata", "loaded state. {}", path.display());
            true
        }
        Err(e) => {
            nds_core::log::error!(target: "nds_core::savedata", "Failed to load state {slot}: {e}");
            false
        }
    }
}

// =========================================================
// ImGui
// =========================================================

#[cfg(target_os = "windows")]
fn setup_imgui(imgui: &mut imgui::Context) {
    let Ok(font_data) = std::fs::read("c:\\Windows\\Fonts\\arial.ttf") else {
        nds_core::log::warn!("Failed to load font, using default");
        return;
    };

    imgui.fonts().add_font(&[imgui::FontSource::TtfData {
        data: &font_data,
        size_pixels: 16.0,
        config: None,
    }]);

    imgui.fonts().build_rgba32_texture();
}

// =========================================================
// UI State (唯一)
// =========================================================

struct UiState {
    show_audio: bool,
}

impl UiState {
    fn new() -> Self {
        Self { show_audio: false }
    }
}

// =========================================================
// Debug State
// =========================================================

struct DebugState {
    palettes: DebugWindow<PalettesWindowState>,
    maps: DebugWindow<MapsWindowState>,
    tiles: DebugWindow<TilesWindowState>,
    vram: DebugWindow<VRAMWindowState>,
    stats: StatsWindow,
}

impl DebugState {
    fn new() -> Self {
        Self {
            palettes: DebugWindow::new("Palettes"),
            maps: DebugWindow::new("Maps"),
            tiles: DebugWindow::new("Tiles"),
            vram: DebugWindow::new("VRAM"),
            stats: StatsWindow::new(),
        }
    }
}

// =========================================================
// Frame Context
// =========================================================

struct FrameCtx {
    nds: NDS,
    config: lunaris_gui_common::config::Config,
    paused: bool,
    menu_height: f32,
}

// =========================================================
// FRAME
// =========================================================

/// 1 frame
fn frame(
    display: &mut Display,
    imgui: &mut imgui::Context,
    ctx: &mut FrameCtx,
    debug: &mut DebugState,
    ui_state: &mut UiState,
) {
    if !ctx.paused {
        ctx.nds.emulate_frame();
        debug.stats.frame_completed();
    }

    let (keys, dropped, state_actions) = display.render_main(&mut ctx.nds, imgui, ctx.menu_height);

    for action in state_actions {
        match action {
            StateAction::Save(slot) => save_state_to_slot(&mut ctx.nds, &ctx.config, slot),
            StateAction::Load(slot) => {
                if load_state_from_slot(&mut ctx.nds, &ctx.config, slot) {
                    ctx.paused = false;
                }
            }
        }
    }

    display.render_imgui(imgui, keys, |ui, keys| {
        render_menu(ctx, ui, debug, ui_state);
        render_debug(&mut ctx.nds, ui, &keys, debug);
        render_audio(&mut ctx.config, &mut ctx.nds, ui, ui_state);
    });

    if dropped.len() == 1 {
        if dropped[0].extension().and_then(|e| e.to_str()) == Some("nds") {
            display.set_last_rom_path(Some(dropped[0].clone()));
            ctx.nds = create_nds(&ctx.config);
            ctx.paused = false;
        }
    }
}

// =========================================================
// Menu
// =========================================================

fn render_menu(ctx: &mut FrameCtx, ui: &Ui, debug: &mut DebugState, ui_state: &mut UiState) {
    ui.main_menu_bar(|| {
        ui.menu(im_str!("File"), true, || {
            if MenuItem::new(im_str!("Open ROM")).build(ui) {
                if let Some(p) = rfd::FileDialog::new().add_filter("NDS ROM", &["nds"]).pick_file()
                {
                    ctx.nds = create_nds(&ctx.config);
                    ctx.paused = false;
                    ctx.config.last_rom_path = Some(p.to_path_buf());
                    ctx.config.save();
                }
            }

            ui.menu(im_str!("Save State"), true, || {
                for i in 1..=5 {
                    if MenuItem::new(imgui::ImString::new(format!("State {i}")).borrow()).build(ui)
                    {
                        save_state_to_slot(&mut ctx.nds, &ctx.config, i);
                    }
                }
            });

            ui.menu(im_str!("Load State"), true, || {
                for i in 1..=5 {
                    if MenuItem::new(imgui::ImString::new(format!("State {i}")).borrow()).build(ui)
                    {
                        if load_state_from_slot(&mut ctx.nds, &ctx.config, i) {
                            ctx.paused = false;
                        }
                    }
                }
            });

            if MenuItem::new(im_str!("Exit")).build(ui) {
                std::process::exit(0);
            }
        });

        ui.menu(im_str!("Emulation"), true, || {
            if MenuItem::new(im_str!("Run")).selected(!ctx.paused).build(ui) {
                ctx.paused = false;
            }

            if MenuItem::new(im_str!("Stop")).selected(ctx.paused).build(ui) {
                ctx.paused = true;
            }

            if MenuItem::new(im_str!("Reset")).build(ui) {
                ctx.nds = create_nds(&ctx.config);
                ctx.paused = false;
            }
        });

        ui.menu(im_str!("Config"), true, || {
            if MenuItem::new(im_str!("Audio")).build(ui) {
                ui_state.show_audio = true;
            }
        });

        ui.menu(im_str!("Debug"), true, || {
            debug.palettes.menu_item(ui);
            debug.maps.menu_item(ui);
            debug.tiles.menu_item(ui);
            debug.vram.menu_item(ui);
            debug.stats.menu_item(ui);
        });

        ctx.menu_height = ui.window_size()[1];
    });
}

// =========================================================
// Debug
// =========================================================

fn render_debug(nds: &mut NDS, ui: &Ui, keys: &HashSet<Key>, debug: &mut DebugState) {
    debug.palettes.render(nds, ui, keys);
    debug.maps.render(nds, ui, keys);
    debug.tiles.render(nds, ui, keys);
    debug.vram.render(nds, ui, keys);
    debug.stats.render(ui);
}

// =========================================================
// Audio
// =========================================================

fn render_audio(
    config: &mut lunaris_gui_common::config::Config,
    nds: &mut NDS,
    ui: &Ui,
    state: &mut UiState,
) {
    if !state.show_audio {
        return;
    }

    Window::new(im_str!("Audio"))
        .opened(&mut state.show_audio)
        .size([300.0, 120.0], Condition::FirstUseEver)
        .build(ui, || {
            if Slider::new(im_str!("Volume")).range(0.0..=100.0).build(ui, &mut config.audio_volume)
            {
                nds.set_audio_volume(config.audio_volume);
                config.save();
            }

            ui.text(format!("{}%", config.audio_volume));
        });
}

// =========================================================
// main
// =========================================================

fn main() {
    setup_logging();

    let mut config = lunaris_gui_common::config::Config::load();
    let rom = resolve_rom_path(&config).expect("ROM required");
    config.last_rom_path = Some(rom.clone());

    let mut imgui = imgui::Context::create();

    #[cfg(target_os = "windows")]
    setup_imgui(&mut imgui);

    let mut display = Display::new(&mut imgui, config.clone());

    let mut debug = DebugState::new();
    let mut ui_state = UiState::new();
    let mut ctx = FrameCtx { nds: create_nds(&config), config, paused: false, menu_height: 0.0 };

    let main_loop = move |display: &mut Display| {
        frame(display, &mut imgui, &mut ctx, &mut debug, &mut ui_state);
    };

    display.run_main_loop(main_loop);
}
