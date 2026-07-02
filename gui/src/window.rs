use crate::NdsGui;
use std::path::PathBuf;

/// Result of GUI rendering
pub enum GuiResult {
    /// User selected a game
    GameSelected(PathBuf),
    /// User cancelled
    Cancelled,
    /// GUI is still open
    Continue,
}

impl NdsGui {
    /// Render the game selection UI
    /// Returns GuiResult indicating what action to take
    pub fn render_imgui(&mut self, ui: &imgui::Ui) -> GuiResult {
        let mut result = GuiResult::Continue;

        imgui::Window::new(imgui::im_str!("NDS Game Selector"))
            .size([400.0, 300.0], imgui::Condition::FirstUseEver)
            .build(ui, || {
                ui.text("Select Folder");
                if ui.button(imgui::im_str!("📁 Browse Folder"), [150.0, 0.0]) {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.set_folder(folder);
                    }
                }

                if let Some(folder) = &self.selected_folder {
                    ui.text(format!(
                        "Folder: {}",
                        folder.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }

                ui.separator();

                ui.text("Select Game");
                let game_names = self.game_names();

                if game_names.is_empty() {
                    if self.selected_folder.is_some() {
                        ui.text_colored([1.0, 0.5, 0.0, 1.0], "No .nds files found");
                    } else {
                        ui.text("No folder selected");
                    }
                } else {
                    // Create combo box for game selection
                    let selected_idx = self
                        .selected_game
                        .as_ref()
                        .and_then(|selected| {
                            game_names
                                .iter()
                                .position(|n| n == selected)
                        })
                        .unwrap_or(0);

                    let mut selected_idx_mut = selected_idx;
                    let game_names_im: Vec<imgui::ImString> = game_names
                        .iter()
                        .map(|s| imgui::ImString::new(s.clone()))
                        .collect();
                    let game_names_refs: Vec<&imgui::ImStr> = 
                        game_names_im.iter().map(|s| s.as_ref()).collect();
                    
                    imgui::ComboBox::new(imgui::im_str!("##game_combo"))
                        .build_simple_string(
                            ui,
                            &mut selected_idx_mut,
                            &game_names_refs,
                        );

                    if selected_idx != selected_idx_mut && selected_idx_mut < game_names.len() {
                        self.selected_game = Some(game_names[selected_idx_mut].clone());
                    }
                }

                ui.separator();

                // Load button
                if ui.button(imgui::im_str!("▶ Load Game"), [100.0, 0.0]) {
                    if let Some(path) = self.get_selected_game_path() {
                        result = GuiResult::GameSelected(path);
                    }
                }

                ui.same_line(110.0);

                // Cancel button
                if ui.button(imgui::im_str!("✕ Cancel"), [100.0, 0.0]) {
                    result = GuiResult::Cancelled;
                }
            });

        result
    }
}
