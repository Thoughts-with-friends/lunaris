// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(feature = "release", windows_subsystem = "windows")]
#![expect(clippy::collapsible_if)]

mod debug;
mod display;
mod input;
mod launcher;
mod savestate;

use std::{borrow::Borrow as _, collections::HashSet, path::PathBuf};

use glfw::Key;
use imgui::{Condition, MenuItem, Slider, Ui, Window, im_str};
use lunaris_gui_common::loader::load_rom;
use nds_core::nds::NDS;

use self::{
    debug::{
        DebugWindow, MapsWindowState, PalettesWindowState, StatsWindow, TilesWindowState,
        VRAMWindowState,
    },
    display::{Display, StateAction},
};

// =========================================================
// ROM
// =========================================================

/// A ROM given on the command line boots straight into the game; without one
/// the app opens on the start screen.
fn cli_rom_path() -> Option<PathBuf> {
    std::env::args().nth(1).map(PathBuf::from).filter(|p| p.exists())
}

/// How many recent ROMs the start screen remembers.
const RECENT_LIMIT: usize = 10;

/// Boots `path`: the single entry point for the start screen, Open ROM and
/// drag-and-drop.
fn load_rom_path(display: &mut Display, ctx: &mut FrameCtx, path: PathBuf) {
    lunaris_rom_library::push_recent(&mut ctx.config.recent_roms, &path, RECENT_LIMIT);
    ctx.config.last_rom_path = Some(path.clone());
    display.set_last_rom_path(Some(path));
    ctx.nds = create_nds(&ctx.config);
    ctx.paused = false;
    ctx.show_launcher = false;
    ctx.config.save();
    ctx.launcher.rescan(&ctx.config.recent_roms, ctx.config.rom_dir.as_deref());
}

fn apply_launcher_action(
    display: &mut Display,
    ctx: &mut FrameCtx,
    action: launcher::LauncherAction,
) {
    match action {
        launcher::LauncherAction::Open(path) => load_rom_path(display, ctx, path),
        launcher::LauncherAction::OpenDialog => {
            if let Some(p) = pick_rom(&ctx.config) {
                load_rom_path(display, ctx, p);
            }
        }
        launcher::LauncherAction::PickFolder => {
            let mut dialog = rfd::FileDialog::new();
            if let Some(dir) = &ctx.config.rom_dir {
                dialog = dialog.set_directory(dir);
            }
            if let Some(dir) = dialog.pick_folder() {
                ctx.config.rom_dir = Some(dir);
                ctx.config.save();
                ctx.launcher.rescan(&ctx.config.recent_roms, ctx.config.rom_dir.as_deref());
            }
        }
    }
}

fn pick_rom(config: &lunaris_gui_common::config::Config) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().add_filter("NDS ROM", &["nds"]);
    if let Some(dir) =
        config.last_rom_path.as_ref().and_then(|p| p.parent()).or(config.rom_dir.as_deref())
    {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_file()
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

    let mut sources =
        vec![imgui::FontSource::TtfData { data: &font_data, size_pixels: 16.0, config: None }];
    // Banner titles on the start screen are mostly Japanese; merge a CJK font.
    let jp_data = std::fs::read("c:\\Windows\\Fonts\\msgothic.ttc").ok();
    if let Some(jp) = &jp_data {
        sources.push(imgui::FontSource::TtfData {
            data: jp,
            size_pixels: 16.0,
            config: Some(imgui::FontConfig {
                glyph_ranges: imgui::FontGlyphRanges::japanese(),
                ..imgui::FontConfig::default()
            }),
        });
    }
    imgui.fonts().add_font(&sources);

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
    /// While set, the start screen covers the window and nothing emulates.
    show_launcher: bool,
    launcher: launcher::Launcher,
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
    if !ctx.paused && !ctx.show_launcher {
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

    let mut launcher_action = None;
    let mut menu_action = None;
    display.render_imgui(imgui, keys, |ui, keys| {
        menu_action = render_menu(ctx, ui, debug, ui_state);
        if ctx.show_launcher {
            launcher_action = ctx.launcher.render(ui, ctx.menu_height);
            if launcher_action.is_none() && ui.io().key_ctrl && keys.contains(&Key::O) {
                launcher_action = Some(launcher::LauncherAction::OpenDialog);
            }
        } else {
            render_debug(&mut ctx.nds, ui, &keys, debug);
        }
        render_audio(&mut ctx.config, &mut ctx.nds, ui, ui_state);
    });
    if let Some(action) = menu_action.or(launcher_action) {
        apply_launcher_action(display, ctx, action);
    }

    if dropped.len() == 1 {
        if dropped[0].extension().and_then(|e| e.to_str()) == Some("nds") {
            load_rom_path(display, ctx, dropped[0].clone());
        }
    }
}

// =========================================================
// Menu
// =========================================================

/// Returns a ROM-loading request (Open ROM) for the caller to apply, since
/// loading needs the [`Display`].
fn render_menu(
    ctx: &mut FrameCtx,
    ui: &Ui,
    debug: &mut DebugState,
    ui_state: &mut UiState,
) -> Option<launcher::LauncherAction> {
    let mut action = None;
    ui.main_menu_bar(|| {
        ui.menu(im_str!("File"), true, || {
            if MenuItem::new(im_str!("Open ROM")).build(ui) {
                action = Some(launcher::LauncherAction::OpenDialog);
            }
            if MenuItem::new(im_str!("Start Screen")).build(ui) {
                ctx.nds.flush_save();
                ctx.show_launcher = true;
                ctx.launcher.rescan(&ctx.config.recent_roms, ctx.config.rom_dir.as_deref());
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
    action
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
    let _ = lunaris_gui_common::log::setup_logging();

    let mut config = lunaris_gui_common::config::Config::load();
    let rom = cli_rom_path();
    let show_launcher = rom.is_none();
    if let Some(rom) = &rom {
        lunaris_rom_library::push_recent(&mut config.recent_roms, rom, RECENT_LIMIT);
        config.last_rom_path = Some(rom.clone());
    }

    let mut imgui = imgui::Context::create();

    #[cfg(target_os = "windows")]
    setup_imgui(&mut imgui);

    let mut display = Display::new(&mut imgui, config.clone());

    let mut debug = DebugState::new();
    let mut ui_state = UiState::new();
    let nds = if show_launcher {
        lunaris_gui_common::loader::placeholder_nds()
    } else {
        create_nds(&config)
    };
    // Icons are GL textures, so the scan runs after `Display::new` made the
    // context current.
    let mut launcher = launcher::Launcher::default();
    launcher.rescan(&config.recent_roms, config.rom_dir.as_deref());
    let mut ctx =
        FrameCtx { nds, config, paused: false, menu_height: 0.0, show_launcher, launcher };

    let main_loop = move |display: &mut Display| {
        frame(display, &mut imgui, &mut ctx, &mut debug, &mut ui_state);
    };

    display.run_main_loop(main_loop);
}
