//! Start screen for the imgui front end: the same game grid as the egui front
//! ends (`lunaris_rom_library::egui_launcher`), drawn with imgui.
use std::path::{Path, PathBuf};

use imgui::{Condition, ImString, TextureId, Ui, Window, WindowFlags, im_str};
use lunaris_rom_library::{ICON_SIZE, RomEntry, scan};

/// What the user asked for on the start screen.
pub enum LauncherAction {
    Open(PathBuf),
    OpenDialog,
    PickFolder,
}

const CARD_W: f32 = 300.0;
const CARD_H: f32 = 72.0;
const GAP: f32 = 12.0;
const ICON_PX: f32 = 48.0;

#[derive(Default)]
pub struct Launcher {
    entries: Vec<RomEntry>,
    /// GL texture names, uploaded once per scan (requires a current context).
    textures: Vec<Option<u32>>,
}

impl Launcher {
    pub fn rescan(&mut self, recents: &[PathBuf], rom_dir: Option<&Path>) {
        self.free_textures();
        self.entries = scan(recents, rom_dir);
        self.textures = self.entries.iter().map(|e| e.icon_rgba.as_deref().map(upload)).collect();
    }

    fn free_textures(&mut self) {
        for tex in self.textures.drain(..).flatten() {
            unsafe { gl::DeleteTextures(1, &tex) };
        }
    }

    /// Draws the start screen below the menu bar.
    pub fn render(&self, ui: &Ui, menu_height: f32) -> Option<LauncherAction> {
        let mut action = None;
        let [w, h] = ui.io().display_size;
        Window::new(im_str!("##start_screen"))
            .position([0.0, menu_height], Condition::Always)
            .size([w, h - menu_height], Condition::Always)
            .flags(
                WindowFlags::NO_DECORATION | WindowFlags::NO_MOVE | WindowFlags::NO_SAVED_SETTINGS,
            )
            .build(ui, || {
                let columns = ((w - 32.0) / (CARD_W + GAP)).floor().clamp(1.0, 3.0) as usize;
                let grid_w = columns as f32 * (CARD_W + GAP) - GAP;
                let left = ((w - grid_w) / 2.0).max(8.0);

                ui.set_cursor_pos([w / 2.0 - 36.0, 24.0]);
                let p = ui.cursor_screen_pos();
                let draw = ui.get_window_draw_list();
                draw.add_rect(p, [p[0] + 72.0, p[1] + 72.0], [0.545, 0.424, 0.965, 1.0])
                    .filled(true)
                    .rounding(16.0)
                    .build();
                draw.add_text([p[0] + 30.0, p[1] + 28.0], [1.0, 1.0, 1.0, 1.0], "L");
                ui.dummy([72.0, 72.0]);

                centered_text(ui, w, "Lunaris DS");
                centered_text(ui, w, "Pick a game below, or open one from a file.");
                ui.dummy([0.0, 8.0]);

                ui.set_cursor_pos([w / 2.0 - 160.0, ui.cursor_pos()[1]]);
                if ui.button(im_str!("Open ROM...  Ctrl+O"), [180.0, 32.0]) {
                    action = Some(LauncherAction::OpenDialog);
                }
                ui.same_line(0.0);
                if ui.button(im_str!("ROM folder..."), [128.0, 32.0]) {
                    action = Some(LauncherAction::PickFolder);
                }
                ui.dummy([0.0, 12.0]);

                ui.set_cursor_pos([left, ui.cursor_pos()[1]]);
                ui.text_disabled("RECENT AND ROM FOLDER");
                if self.entries.is_empty() {
                    ui.set_cursor_pos([left, ui.cursor_pos()[1]]);
                    ui.text("No ROMs yet. Choose a ROM folder to list its games here.");
                }
                for (i, entry) in self.entries.iter().enumerate() {
                    let col = i % columns;
                    if col == 0 {
                        ui.set_cursor_pos([left, ui.cursor_pos()[1]]);
                    } else {
                        ui.same_line_with_spacing(0.0, GAP);
                    }
                    if self.card(ui, i, entry) {
                        action = Some(LauncherAction::Open(entry.path.clone()));
                    }
                }
            });
        action
    }

    fn card(&self, ui: &Ui, index: usize, entry: &RomEntry) -> bool {
        let p = ui.cursor_screen_pos();
        let max = [p[0] + CARD_W, p[1] + CARD_H];
        let hovered = ui.is_mouse_hovering_rect(p, max) && ui.is_window_hovered();
        let icon_min = [p[0] + 12.0, p[1] + (CARD_H - ICON_PX) / 2.0];
        let icon_max = [icon_min[0] + ICON_PX, icon_min[1] + ICON_PX];
        let texture = self.textures.get(index).copied().flatten();
        {
            let draw = ui.get_window_draw_list();
            let fill = if hovered { [0.19, 0.19, 0.26, 1.0] } else { [0.14, 0.14, 0.19, 1.0] };
            draw.add_rect(p, max, fill).filled(true).rounding(8.0).build();
            draw.add_rect(p, max, [0.3, 0.3, 0.3, 1.0]).rounding(8.0).build();
            if texture.is_none() {
                draw.add_rect(icon_min, icon_max, [0.23, 0.25, 0.56, 1.0])
                    .filled(true)
                    .rounding(6.0)
                    .build();
                draw.add_text([icon_min[0] + 15.0, icon_min[1] + 16.0], [1.0, 1.0, 1.0, 1.0], "DS");
            }
            let text_x = icon_max[0] + 12.0;
            draw.add_text([text_x, p[1] + 16.0], [1.0, 1.0, 1.0, 1.0], &entry.title);
            draw.add_text([text_x, p[1] + 40.0], [0.6, 0.6, 0.6, 1.0], &entry.file_name);
        }
        if let Some(tex) = texture {
            ui.set_cursor_screen_pos(icon_min);
            imgui::Image::new(TextureId::from(tex as usize), [ICON_PX, ICON_PX]).build(ui);
            ui.set_cursor_screen_pos(p);
        }
        // The card-wide button goes last so it is the item `same_line` follows.
        let clicked =
            ui.invisible_button(&ImString::new(format!("##rom{index}")), [CARD_W, CARD_H]);
        if ui.is_item_hovered() {
            ui.tooltip_text(entry.path.display().to_string());
        }
        clicked
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        self.free_textures();
    }
}

fn centered_text(ui: &Ui, width: f32, text: &str) {
    let size = ui.calc_text_size(&ImString::new(text), false, -1.0);
    ui.set_cursor_pos([(width - size[0]) / 2.0, ui.cursor_pos()[1]]);
    ui.text(text);
}

fn upload(rgba: &[u8]) -> u32 {
    let mut tex = 0;
    unsafe {
        gl::GenTextures(1, &mut tex);
        gl::BindTexture(gl::TEXTURE_2D, tex);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA as i32,
            ICON_SIZE as i32,
            ICON_SIZE as i32,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            rgba.as_ptr().cast(),
        );
    }
    tex
}
