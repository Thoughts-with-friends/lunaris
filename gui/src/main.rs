// Prevents additional console window on Windows in release, DO NOT REMOVE!!
// #![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![expect(clippy::collapsible_if)]
mod config;
mod debug;
mod display;
mod input;

use std::fs::{self, File};
use std::path::PathBuf;

use nds_core::log::*;
use nds_core::nds::NDS;
use nds_core::simplelog::*;

use debug::*;
use display::Display;
use imgui::*;

fn setup_logging() {
    let arm7_file_name = "ROMs/arm7.log";
    let arm9_file_name = "ROMs/arm9.log";
    let savedata_file_name = "ROMs/savedata.log";

    let instructions7_filter = LevelFilter::Off;
    let instructions9_filter = LevelFilter::Off;
    let savedata_filter = LevelFilter::Off;

    let arm7_file = File::create(arm7_file_name);
    let arm9_file = File::create(arm9_file_name);
    let savedata_file = File::create(savedata_file_name);

    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![TermLogger::new(
        LevelFilter::Off,
        // LevelFilter::Warn,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )];

    if let Ok(file) = arm7_file {
        loggers.push(WriteLogger::new(
            instructions7_filter,
            ConfigBuilder::new()
                .set_time_level(LevelFilter::Off)
                .set_thread_level(LevelFilter::Off)
                .set_target_level(LevelFilter::Off)
                .set_location_level(LevelFilter::Off)
                .set_time_level(LevelFilter::Off)
                .set_max_level(LevelFilter::Off)
                .add_filter_allow_str("nds_core::arm7")
                .build(),
            file,
        ));
    }

    if let Ok(file) = arm9_file {
        loggers.push(WriteLogger::new(
            instructions9_filter,
            ConfigBuilder::new()
                .set_time_level(LevelFilter::Off)
                .set_thread_level(LevelFilter::Off)
                .set_target_level(LevelFilter::Off)
                .set_location_level(LevelFilter::Off)
                .set_time_level(LevelFilter::Off)
                .set_max_level(LevelFilter::Off)
                .add_filter_allow_str("nds_core::arm9")
                .build(),
            file,
        ));
    }

    if let Ok(file) = savedata_file {
        loggers.push(WriteLogger::new(
            savedata_filter,
            ConfigBuilder::new()
                // .set_time_level(LevelFilter::Off)
                // .set_thread_level(LevelFilter::Off)
                // .set_target_level(LevelFilter::Off)
                // .set_location_level(LevelFilter::Off)
                // .set_max_level(LevelFilter::Off)
                .add_filter_allow_str("nds_core::savedata")
                .build(),
            file,
        ));
    }

    CombinedLogger::init(loggers).unwrap();
}

fn main() {
    // Setup logging first
    setup_logging();

    // Try to get ROM path from command line or show file selection dialog
    let mut config = self::config::Config::load();
    let mut current_rom_path = if let Some(arg) = std::env::args().nth(1)
        && PathBuf::from(arg.as_str()).exists()
    {
        Some(PathBuf::from(arg))
    } else if let Some(path) = config.last_rom_path.clone()
        && path.exists()
    {
        Some(path)
    } else {
        match rfd::FileDialog::new()
            .add_filter("NDS ROM", &["nds"])
            .pick_file()
        {
            Some(path) => {
                config.last_rom_path = Some(path.clone());
                Some(path)
            }
            None => {
                error!("No ROM file selected");
                std::process::exit(1);
            }
        }
    };

    let rom_path = current_rom_path.clone().unwrap();

    let bios7_path = PathBuf::from("ROMs/bios7.bin");
    let bios9_path = PathBuf::from("ROMs/bios9.bin");
    let firmware_path = PathBuf::from("ROMs/firmware.bin");

    let bios7_path = bios7_path.exists().then_some(bios7_path);
    let bios7_path = bios7_path.as_deref();

    let bios9_path = bios9_path.exists().then_some(bios9_path);
    let bios9_path = bios9_path.as_deref();

    let firmware_path = firmware_path.exists().then_some(firmware_path);
    let firmware_path = firmware_path.as_deref();

    let mut nds = NDS::load_rom(
        bios7_path,
        bios9_path,
        firmware_path,
        &rom_path,
        config.audio_volume,
    );

    let mut imgui = Context::create();
    let mut display = Display::new(&mut imgui, config);

    // =========================================================
    // CHANGE:
    // Added emulator execution state
    // =========================================================
    let mut paused = false;

    let mut main_menu_height = 0.0;

    let mut palettes_window = DebugWindow::<PalettesWindowState>::new("Palettes");

    let mut maps_window = DebugWindow::<MapsWindowState>::new("Maps");

    let mut tiles_window = DebugWindow::<TilesWindowState>::new("Tiles");

    let mut vram_window = DebugWindow::<VRAMWindowState>::new("VRAM");

    let mut stats_window = StatsWindow::new();

    let mut show_audio_settings_window = false;

    let main_loop = move |display: &mut Display| {
        // =====================================================
        // CHANGE:
        // Only emulate frames when not paused
        // =====================================================
        if !paused {
            nds.emulate_frame();
            stats_window.frame_completed();
        }

        // Normal rendering with ROM
        let (keys_pressed, files_dropped) =
            display.render_main(&mut nds, &mut imgui, main_menu_height);

        let mut pending_last_rom_path: Option<PathBuf> = None;
        let mut audio_volume = display.audio_volume();
        let mut audio_volume_changed = false;

        display.render_imgui(&mut imgui, keys_pressed, |ui, keys_pressed| {
            ui.main_menu_bar(|| {
                // =================================================
                // CHANGE:
                // Added File menu
                // =================================================
                ui.menu(im_str!("File"), true, || {
                    // =============================================
                    // CHANGE:
                    // Open ROM from file dialog
                    // =============================================
                    if MenuItem::new(im_str!("Open ROM")).build(ui) {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("NDS ROM", &["nds"])
                            .pick_file()
                        {
                            current_rom_path = Some(path.clone());
                            pending_last_rom_path = Some(path.clone());
                            nds = NDS::load_rom(
                                bios7_path,
                                bios9_path,
                                firmware_path,
                                &path,
                                audio_volume,
                            );
                            paused = false;

                            info!("Loaded ROM: {:?}", path);
                        }
                    }

                    if MenuItem::new(im_str!("Import Savefile")).build(ui) {
                        if let Some(save_path) = rfd::FileDialog::new()
                            .add_filter("NDS Save", &["sav"])
                            .pick_file()
                        {
                            if let Some(rom_path) = current_rom_path.clone() {
                                let target_save_path = rom_path.with_extension("sav");
                                if let Err(err) = fs::copy(&save_path, &target_save_path) {
                                    error!("Failed to import savefile: {}", err);
                                } else {
                                    nds = NDS::load_rom(
                                        bios7_path,
                                        bios9_path,
                                        firmware_path,
                                        &rom_path,
                                        audio_volume,
                                    );
                                    paused = false;
                                    info!("Imported savefile from {:?}", save_path);
                                }
                            } else {
                                error!("Cannot import savefile without a loaded ROM");
                            }
                        }
                    }

                    // =============================================
                    // CHANGE:
                    // Exit menu item
                    // =============================================
                    if MenuItem::new(im_str!("Exit")).build(ui) {
                        std::process::exit(0);
                    }
                });

                // =================================================
                // CHANGE:
                // Added emulation control menu
                // =================================================
                ui.menu(im_str!("Emulation"), true, || {
                    // Run emulator
                    if MenuItem::new(im_str!("Run")).selected(!paused).build(ui) {
                        paused = false;
                    }

                    // Pause emulator
                    if MenuItem::new(im_str!("Stop")).selected(paused).build(ui) {
                        paused = true;
                    }

                    // =============================================
                    // OPTIONAL:
                    // Reset emulator
                    // =============================================
                    if MenuItem::new(im_str!("Reset")).build(ui) {
                        if let Some(ref rom_path) = current_rom_path {
                            nds = NDS::load_rom(
                                bios7_path,
                                bios9_path,
                                firmware_path,
                                rom_path,
                                audio_volume,
                            );
                            paused = false;
                        } else {
                            error!("Cannot reset emulator without a loaded ROM");
                        }
                    }
                });

                // Existing debug menu
                ui.menu(im_str!("Debug Windows"), true, || {
                    palettes_window.menu_item(ui);
                    maps_window.menu_item(ui);
                    tiles_window.menu_item(ui);
                    vram_window.menu_item(ui);
                    stats_window.menu_item(ui);
                });

                // =================================================
                // Added Config menu
                // =================================================
                ui.menu(im_str!("Config"), true, || {
                    if MenuItem::new(im_str!("Audio Settings")).build(ui) {
                        show_audio_settings_window = true;
                    }
                });

                main_menu_height = ui.window_size()[1];
            });

            if show_audio_settings_window {
                imgui::Window::new(imgui::im_str!("Audio Settings"))
                    .size([320.0, 110.0], imgui::Condition::FirstUseEver)
                    .opened(&mut show_audio_settings_window)
                    .build(ui, || {
                        if Slider::new(imgui::im_str!("Volume"))
                            .range(0.0_f32..=100.0_f32)
                            .build(ui, &mut audio_volume)
                        {
                            audio_volume_changed = true;
                        }
                        ui.text(format!("Volume: {:.0}%", audio_volume));
                    });
            }

            palettes_window.render(&mut nds, ui, &keys_pressed);

            maps_window.render(&mut nds, ui, &keys_pressed);

            tiles_window.render(&mut nds, ui, &keys_pressed);

            vram_window.render(&mut nds, ui, &keys_pressed);

            stats_window.render(ui);
        });

        if let Some(path) = pending_last_rom_path.take() {
            display.set_last_rom_path(Some(path));
        }

        if audio_volume_changed {
            display.set_audio_volume(audio_volume);
            nds.set_audio_volume(audio_volume);
            display.save_config();
        }

        // =====================================================
        // Existing drag-and-drop ROM loading
        // =====================================================
        if files_dropped.len() == 1 {
            if let Some(ext) = files_dropped[0].extension() {
                if let Some(str) = ext.to_str() {
                    if str.to_lowercase() == "nds" {
                        nds = NDS::load_rom(
                            bios7_path,
                            bios9_path,
                            firmware_path,
                            &files_dropped[0],
                            audio_volume,
                        );

                        // =========================================
                        // CHANGE:
                        // Resume after drag-and-drop ROM load
                        // =========================================
                        paused = false;

                        info!("Loaded ROM from drag-and-drop: {:?}", files_dropped[0]);
                    } else {
                        error!("File is not a .nds file!");
                    }
                }
            } else {
                error!("File does not have an extension!");
            }
        } else if files_dropped.len() > 1 {
            error!("More than 1 file dropped!");
        }
    };

    display.run_main_loop(main_loop);
}
