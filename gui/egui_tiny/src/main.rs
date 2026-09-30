//! Minimal egui front end for [`core_base`].
//!
//! Starts on an empty screen with a menu bar; `File -> Open ROM...` picks a
//! `.nds` file and begins emulation immediately. The two DS screens are drawn
//! stacked vertically, the lower one accepting the stylus.

mod input;
mod screen;

use core_base::Nds;
use input::Input;
use screen::Screens;

/// Application state: the console, if one has been loaded, plus its textures.
#[derive(Default)]
struct App {
    nds: Option<Nds>,
    screens: Screens,
    input: Input,
    /// Message shown before a ROM is loaded, or after a failed load.
    status: String,
}

impl App {
    /// Opens a file picker and boots the chosen ROM.
    fn open_rom(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Nintendo DS ROM", &["nds"]).pick_file()
        else {
            return;
        };
        self.boot(&path);
    }

    /// Boots `path`, replacing any console already running.
    fn boot(&mut self, path: &std::path::Path) {
        // Dropped first so the outgoing console flushes its save file before
        // the incoming one opens the same `.sav`.
        self.nds = None;
        match Nds::new(path) {
            Ok(nds) => {
                self.nds = Some(nds);
                self.status = format!("Running {}", path.display());
            }
            Err(error) => {
                self.nds = None;
                self.status = format!("Could not boot {}: {error}", path.display());
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open ROM...").clicked() {
                        ui.close();
                        self.open_rom();
                    }
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.separator();
                ui.label(&self.status);
                if let Some(name) = self.input.gamepad_name() {
                    ui.separator();
                    ui.label(format!("Gamepad: {name}"));
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(nds) = self.nds.as_mut() else {
                ui.centered_and_justified(|ui| ui.label("Open a ROM to start emulation."));
                return;
            };
            self.input.apply(ctx, nds);
            nds.run_frame();
            self.screens.show(ui, nds);
            // Emulation is paced by the display refresh, so ask for the next
            // frame as soon as this one is on screen.
            ctx.request_repaint();
        });
    }
}

/// Runs the front end. A path given on the command line boots straight away,
/// skipping the file picker.
fn main() -> eframe::Result {
    let rom = std::env::args().nth(1);
    eframe::run_native(
        "core_base",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([512.0, 800.0]),
            ..Default::default()
        },
        Box::new(move |_| {
            let mut app = App::default();
            if let Some(rom) = rom {
                app.boot(std::path::Path::new(&rom));
            }
            Ok(Box::new(app))
        }),
    )
}
