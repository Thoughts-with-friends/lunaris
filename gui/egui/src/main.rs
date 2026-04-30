mod config;

use self::config::{Config, load_config, save_config};
use eframe::egui;
use lunaris_ds_emu::Emulator;
use lunaris_ds_mem_const::{PIXELS_PER_LINE, SCANLINES};
use std::{path::PathBuf, time::Instant};

// ===== util =====
fn list_nds_files(dir: &std::path::Path) -> Vec<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("nds"))
            })
            .collect()
    } else {
        vec![]
    }
}

// ===== app =====
struct App {
    config: Config,
    emu: Option<Emulator>,
    is_running: bool,

    rom_list: Vec<PathBuf>,

    upper_tex: Option<egui::TextureHandle>,
    lower_tex: Option<egui::TextureHandle>,

    last_time: Instant,
    frame_count: u64,
    fps: f32,
    rom_list_open: bool,
}

impl App {
    fn new() -> Self {
        let config = load_config();
        let rom_list = list_nds_files(&config.rom_dir);

        Self {
            config,
            emu: None,
            is_running: false,
            rom_list,
            upper_tex: None,
            lower_tex: None,
            last_time: Instant::now(),
            frame_count: 0,
            fps: 0.0,
            rom_list_open: false,
        }
    }

    // ===== UI =====
    fn ui_top(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("ROM DIR:");

                let mut dir_str = self.config.rom_dir.to_string_lossy().to_string();
                if ui.text_edit_singleline(&mut dir_str).changed() {
                    self.config.rom_dir = dir_str.into();
                    self.rom_list = list_nds_files(&self.config.rom_dir);
                }
            });

            ui.separator();

            self.ui_rom_list(ui);

            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("Run").clicked() {
                    self.run_emulator();
                }

                if ui.button("Stop").clicked() {
                    self.stop_emulator();
                }
            });

            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut self.config.scale, 1.0..=5.0).text("Scale"));
                ui.checkbox(&mut self.config.show_fps, "FPS");
            });
        });
    }

    fn ui_rom_list(&mut self, ui: &mut egui::Ui) {
        ui.separator();

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                let label = if self.rom_list_open {
                    "ROM List ▼"
                } else {
                    "ROM List ▶"
                };

                if ui.button(label).clicked() {
                    self.rom_list_open = !self.rom_list_open;
                }

                if self.rom_list_open {
                    egui::ScrollArea::vertical()
                        .max_height(150.0)
                        .show(ui, |ui| {
                            for path in &self.rom_list {
                                let name = path.file_name().unwrap_or_default().to_string_lossy();

                                let selected =
                                    self.config.selected_rom.as_ref().is_some_and(|p| p == path);

                                let resp = ui.selectable_label(selected, name);

                                if resp.clicked() {
                                    self.config.selected_rom = Some(path.clone());
                                }
                            }
                        });
                }
            });

            ui.separator();

            ui.vertical(|ui| {
                ui.label("Selected:");

                if let Some(path) = &self.config.selected_rom {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();

                    ui.monospace(name);

                    ui.small(path.to_string_lossy());
                } else {
                    ui.label("None");
                }
            });
        });
    }

    fn ui_emulator(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.is_running {
                if let Some(emu) = &mut self.emu {
                    Self::render_frame(
                        ctx,
                        ui,
                        emu,
                        &mut self.upper_tex,
                        &mut self.lower_tex,
                        self.config.scale,
                    );
                    self.update_fps(ctx);
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a ROM and press Run");
                });
            }
        });
    }

    // ===== emulator control =====
    fn run_emulator(&mut self) {
        let Some(path) = &self.config.selected_rom else {
            eprintln!("No ROM selected");
            return;
        };

        let mut emu = Emulator::new();

        match emu.load_rom(path) {
            Ok(_) => {
                self.emu = Some(emu);
                self.is_running = true;
                save_config(&self.config);
            }
            Err(e) => {
                eprintln!("Failed to load ROM: {:?}", e);
                self.is_running = false;
            }
        }
    }

    fn stop_emulator(&mut self) {
        self.is_running = false;
        self.emu = None;
    }

    // ===== rendering =====
    fn render_frame(
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        emu: &mut Emulator,
        upper_tex: &mut Option<egui::TextureHandle>,
        lower_tex: &mut Option<egui::TextureHandle>,
        scale: f32,
    ) {
        const PIXEL: usize = PIXELS_PER_LINE * SCANLINES;

        let mut upper_buffer = vec![0_u32; PIXEL];
        let mut lower_buffer = vec![0_u32; PIXEL];

        emu.run();
        emu.get_upper_frame(&mut upper_buffer);
        emu.get_lower_frame(&mut lower_buffer);

        let upper_img = egui::ColorImage::from_rgba_unmultiplied(
            [PIXELS_PER_LINE, SCANLINES],
            bytemuck::cast_slice(&upper_buffer),
        );

        let lower_img = egui::ColorImage::from_rgba_unmultiplied(
            [PIXELS_PER_LINE, SCANLINES],
            bytemuck::cast_slice(&lower_buffer),
        );

        Self::update_texture(ctx, upper_tex, "upper", upper_img);
        Self::update_texture(ctx, lower_tex, "lower", lower_img);

        ui.vertical_centered(|ui| {
            if let Some(tex) = upper_tex.as_ref() {
                ui.add(egui::Image::new(tex).fit_to_exact_size(tex.size_vec2() * scale));
            }

            ui.add_space(8.0);

            if let Some(tex) = lower_tex.as_ref() {
                ui.add(egui::Image::new(tex).fit_to_exact_size(tex.size_vec2() * scale));
            }
        });
    }

    fn update_texture(
        ctx: &egui::Context,
        tex_slot: &mut Option<egui::TextureHandle>,
        name: &str,
        img: egui::ColorImage,
    ) {
        if let Some(tex) = tex_slot {
            tex.set(img, egui::TextureOptions::NEAREST);
        } else {
            *tex_slot = Some(ctx.load_texture(name, img, egui::TextureOptions::NEAREST));
        }
    }

    // ===== fps =====
    fn update_fps(&mut self, ctx: &egui::Context) {
        self.frame_count += 1;

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_time).as_secs_f32();

        if elapsed >= 1.0 {
            self.fps = self.frame_count as f32 / elapsed;
            self.frame_count = 0;
            self.last_time = now;
        }

        if self.config.show_fps {
            egui::Area::new("fps_overlay".into())
                .fixed_pos([10.0, 10.0])
                .show(ctx, |ui| {
                    ui.label(format!("FPS: {:.1}", self.fps));
                });
        }
    }
}

// ===== eframe =====
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui_top(ctx);
        self.ui_emulator(ctx);
        ctx.request_repaint();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        save_config(&self.config);
    }
}

// ===== entry =====
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();

    eframe::run_native(
        "NDS Emulator",
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
