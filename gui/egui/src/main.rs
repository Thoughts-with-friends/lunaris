mod config;

use self::config::{Config, load_config, save_config};
use eframe::egui;
use lunaris_ds_emu::Emulator;
use lunaris_ds_mem_const::{PIXELS_PER_LINE, SCANLINES};
use std::time::Instant;

struct App {
    config: Config,
    emu: Option<Emulator>,
    is_running: bool,

    upper_tex: Option<egui::TextureHandle>,
    lower_tex: Option<egui::TextureHandle>,

    // fps
    last_time: Instant,
    frame_count: u64,
    fps: f32,
}

impl App {
    fn new() -> Self {
        Self {
            config: load_config(),
            emu: None,
            is_running: false,
            upper_tex: None,
            lower_tex: None,
            last_time: Instant::now(),
            frame_count: 0,
            fps: 0.0,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ===== Top UI =====
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("ROM:");

                let mut path_str = self.config.rom_path.to_string_lossy().to_string();
                if ui.text_edit_singleline(&mut path_str).changed() {
                    self.config.rom_path = path_str.into();
                }

                if ui.button("Run").clicked() {
                    let mut emu = Emulator::new();

                    match emu.load_rom(&self.config.rom_path) {
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

                if ui.button("Stop").clicked() {
                    self.is_running = false;
                    self.emu = None;
                }
            });

            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut self.config.scale, 1.0..=5.0).text("Scale"));

                ui.checkbox(&mut self.config.show_fps, "FPS");
            });
        });

        // ===== Emulator & Rendering =====
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.is_running {
                if let Some(emu) = &mut self.emu {
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

                    if let Some(tex) = &mut self.upper_tex {
                        tex.set(upper_img, egui::TextureOptions::NEAREST);
                    } else {
                        self.upper_tex = Some(ctx.load_texture(
                            "upper",
                            upper_img,
                            egui::TextureOptions::NEAREST,
                        ));
                    }

                    if let Some(tex) = &mut self.lower_tex {
                        tex.set(lower_img, egui::TextureOptions::NEAREST);
                    } else {
                        self.lower_tex = Some(ctx.load_texture(
                            "lower",
                            lower_img,
                            egui::TextureOptions::NEAREST,
                        ));
                    }

                    let scale = self.config.scale;

                    ui.vertical_centered(|ui| {
                        if let Some(tex) = &self.upper_tex {
                            ui.add(
                                egui::Image::new(tex).fit_to_exact_size(tex.size_vec2() * scale),
                            );
                        }

                        ui.add_space(8.0);

                        if let Some(tex) = &self.lower_tex {
                            ui.add(
                                egui::Image::new(tex).fit_to_exact_size(tex.size_vec2() * scale),
                            );
                        }
                    });

                    // ===== FPS =====
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
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Load a ROM and press Run");
                });
            }
        });

        ctx.request_repaint();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        save_config(&self.config);
    }
}

// ===== entry point =====
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();

    eframe::run_native(
        "NDS Emulator",
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
