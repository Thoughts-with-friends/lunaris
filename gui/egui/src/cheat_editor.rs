use egui::Context;
use nds_core::CheatMap;

use crate::config::Config;

/// State manager for the Action Replay (AR) cheat code editor UI.
///
/// This structure handles the multi-line text input buffer, visibility toggles,
/// and status communication back to the user regarding parse or injection results.
#[derive(Debug, Default, Clone)]
pub struct CheatEditorState {
    /// Determines whether the editor window is currently visible.
    pub is_open: bool,
    /// Raw text buffer containing the multi-line AR input string.
    pub text_buffer: String,
    /// Feedback message displayed at the bottom of the editor (e.g., "OK" or "Err").
    pub status_message: String,
    /// Flag indicating whether the current status message represents an error condition.
    pub is_error: bool,
}

impl CheatEditorState {
    /// Renders the cheat editor window and dispatches data mutations to the provided memory buffer.
    ///
    /// This function constructs a vertical layout where the top section acts as a flexible,
    /// monospace text editor, and the bottom section hosts the submission control paired with
    /// a colored diagnostic label matching your layout constraints.
    ///
    /// # Layout Structure
    /// ```text
    /// +---------------------------------------+
    /// | AR Cheat Code Editor                  |
    /// +---------------------------------------+
    /// | [ Multi-line Monospace Text Buffer ]   |
    /// | |                                   | |
    /// | |                                   | |
    /// +---------------------------------------+
    /// | [Apply] | Message Box: OK or Err      |
    /// +---------------------------------------+
    /// ```
    ///
    /// # Arguments
    ///
    /// * `ctx` - A reference to the current `egui::Context` topology.
    /// * `main_mem` - A mutable slice representing the emulator's raw 4 MiB Main RAM buffer.
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Inside your main egui rendering loop:
    /// cheat_editor.show_ui(ctx, &mut self.hw.main_mem);
    /// ```
    pub fn show_cheats(&mut self, ctx: &Context, config: &Config) -> Option<CheatMap> {
        let mut cheat_map = None;

        egui::Window::new("AR Cheat Code Editor")
            .open(&mut self.is_open)
            .default_width(380.0)
            .resizable(true)
            .max_height(700.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_sized(
                        [ui.available_width(), 300.0],
                        egui::TextEdit::multiline(&mut self.text_buffer)
                            .hint_text("Enter AR codes here...\nExample:\n9223DD32 0000319C\n0223DD34 60080180")
                            .font(egui::TextStyle::Monospace)
                    );
                });

                ui.separator();

                // Footer section rendering controls and the status log layout.
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        if self.text_buffer.is_empty() {
                            self.status_message = "Err: Code buffer is empty".to_string();
                            self.is_error = true;
                        } else {
                            // TODO: Integrate the structural iterator for CheatOp execution here.
                            self.status_message = "OK: Applied successfully".to_string();
                            match lunaris_gui_common::cheat_map::cheat_map_from_str(&self.text_buffer) {
                                Ok(map) => {
                                    cheat_map = Some(map);
                                    write_cheat_map(config, &self.text_buffer);
                                    self.is_error = false;
                                }
                                Err(err) => {
                                    self.is_error = true;
                                    self.status_message = err.to_string();
                                },
                            };
                        }
                    }

                    ui.separator();

                    // Apply visual accents directly matching the error state context.
                    let color = if self.is_error {
                        egui::Color32::LIGHT_RED
                    } else if self.status_message.starts_with("OK") {
                        egui::Color32::LIGHT_GREEN
                    } else {
                        ui.style().visuals.text_color()
                    };

                    ui.colored_label(color, &self.status_message);
                });
            });

        cheat_map
    }
}

fn write_cheat_map(config: &Config, cheat_txt: &str) {
    if let Some(rom_name) = config.last_rom_path.as_ref().and_then(|p| p.file_name()) {
        let cheat_dir = config.cheat_dir.as_path();
        let mut cheat_file = cheat_dir.join(rom_name);
        cheat_file.set_extension("txt");
        let _ = std::fs::create_dir_all(cheat_dir);
        let _ = std::fs::write(cheat_file, cheat_txt);
    }
}
