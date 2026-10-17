//! The start screen for the egui front ends: logo, "Open ROM…" / "ROM
//! folder…" buttons and a grid of game cards (banner icon, title, file name).
use std::path::{Path, PathBuf};

use egui::{Color32, CornerRadius, RichText, Stroke, TextureHandle, Vec2};

use crate::{ICON_SIZE, RomEntry, scan};

/// What the user asked for on the start screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherAction {
    /// A game card was clicked.
    Open(PathBuf),
    /// "Open ROM…" (or Ctrl+O).
    OpenDialog,
    /// "ROM folder…": the front end picks a folder, stores it and rescans.
    PickFolder,
}

/// User-visible strings, so translated front ends can pass their own.
#[derive(Debug, Clone)]
pub struct LauncherText {
    pub title: String,
    pub subtitle: String,
    pub open_rom: String,
    pub rom_folder: String,
    pub section: String,
    pub empty: String,
}

impl Default for LauncherText {
    fn default() -> Self {
        Self {
            title: "Lunaris DS".into(),
            subtitle: "Pick a game below, or open one from a file.".into(),
            open_rom: "Open ROM...    Ctrl+O".into(),
            rom_folder: "ROM folder...".into(),
            section: "RECENT AND ROM FOLDER".into(),
            empty: "No ROMs yet. Choose a ROM folder to list its games here.".into(),
        }
    }
}

const ACCENT: Color32 = Color32::from_rgb(0x8B, 0x6C, 0xF6);
const CARD: Color32 = Color32::from_rgb(0x24, 0x24, 0x30);
const CARD_HOVER: Color32 = Color32::from_rgb(0x30, 0x30, 0x42);
const CARD_WIDTH: f32 = 300.0;
const CARD_HEIGHT: f32 = 72.0;
const ICON_PX: f32 = 48.0;

/// Game list plus the icon textures uploaded for it.
#[derive(Default)]
pub struct Launcher {
    entries: Vec<RomEntry>,
    icons: Vec<Option<TextureHandle>>,
    uploaded: bool,
}

impl Launcher {
    /// Rebuilds the list from `recents` and `rom_dir` (call at startup and
    /// whenever either changes, not every frame).
    pub fn rescan(&mut self, recents: &[PathBuf], rom_dir: Option<&Path>) {
        self.entries = scan(recents, rom_dir);
        self.icons.clear();
        self.uploaded = false;
    }

    pub fn entries(&self) -> &[RomEntry] {
        &self.entries
    }

    fn upload_icons(&mut self, ctx: &egui::Context) {
        if self.uploaded {
            return;
        }
        self.icons = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                e.icon_rgba.as_ref().map(|rgba| {
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied([ICON_SIZE, ICON_SIZE], rgba);
                    ctx.load_texture(format!("rom_icon_{i}"), image, egui::TextureOptions::NEAREST)
                })
            })
            .collect();
        self.uploaded = true;
    }

    /// Draws the start screen filling `ui`, returning the user's choice.
    pub fn show(&mut self, ui: &mut egui::Ui, text: &LauncherText) -> Option<LauncherAction> {
        self.upload_icons(ui.ctx());
        let mut action = None;
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            action = Some(LauncherAction::OpenDialog);
        }

        let columns =
            ((ui.available_width() - 32.0) / (CARD_WIDTH + 12.0)).floor().clamp(1.0, 3.0) as usize;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(24.0);
                logo(ui);
                ui.add_space(8.0);
                ui.label(RichText::new(&text.title).size(30.0).strong().color(Color32::WHITE));
                ui.label(RichText::new(&text.subtitle).color(Color32::LIGHT_GRAY));
                ui.add_space(12.0);

                let buttons_width = 320.0;
                ui.allocate_ui(Vec2::new(buttons_width, 36.0), |ui| {
                    ui.horizontal(|ui| {
                        let open = egui::Button::new(
                            RichText::new(&text.open_rom).size(16.0).color(Color32::WHITE),
                        )
                        .fill(ACCENT)
                        .min_size(Vec2::new(0.0, 32.0));
                        if ui.add(open).clicked() {
                            action = Some(LauncherAction::OpenDialog);
                        }
                        let folder = egui::Button::new(RichText::new(&text.rom_folder).size(16.0))
                            .min_size(Vec2::new(0.0, 32.0));
                        if ui.add(folder).clicked() {
                            action = Some(LauncherAction::PickFolder);
                        }
                    });
                });
                ui.add_space(16.0);

                let grid_width = columns as f32 * (CARD_WIDTH + 12.0) - 12.0;
                ui.allocate_ui(Vec2::new(grid_width, 0.0), |ui| {
                    ui.label(RichText::new(&text.section).small().color(Color32::GRAY));
                    if self.entries.is_empty() {
                        ui.label(&text.empty);
                        return;
                    }
                    egui::Grid::new("rom_launcher_grid").spacing([12.0, 10.0]).show(ui, |ui| {
                        for (i, entry) in self.entries.iter().enumerate() {
                            let icon = self.icons.get(i).and_then(Option::as_ref);
                            if card(ui, entry, icon).clicked() {
                                action = Some(LauncherAction::Open(entry.path.clone()));
                            }
                            if (i + 1) % columns == 0 {
                                ui.end_row();
                            }
                        }
                    });
                });
                ui.add_space(24.0);
            });
        });
        action
    }
}

fn logo(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(72.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(16), ACCENT);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "L",
        egui::FontId::proportional(40.0),
        Color32::WHITE,
    );
}

fn card(ui: &mut egui::Ui, entry: &RomEntry, icon: Option<&TextureHandle>) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(CARD_WIDTH, CARD_HEIGHT), egui::Sense::click());
    let fill = if response.hovered() { CARD_HOVER } else { CARD };
    let painter = ui.painter_at(rect);
    painter.rect(
        rect,
        CornerRadius::same(8),
        fill,
        Stroke::new(1.0_f32, Color32::from_gray(70)),
        egui::StrokeKind::Inside,
    );

    let icon_rect = egui::Rect::from_min_size(
        rect.min + Vec2::new(12.0, (CARD_HEIGHT - ICON_PX) / 2.0),
        Vec2::splat(ICON_PX),
    );
    match icon {
        Some(tex) => {
            painter.image(
                tex.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(
                icon_rect,
                CornerRadius::same(6),
                Color32::from_rgb(0x3A, 0x3F, 0x8F),
            );
            painter.text(
                icon_rect.center(),
                egui::Align2::CENTER_CENTER,
                "DS",
                egui::FontId::proportional(18.0),
                Color32::WHITE,
            );
        }
    }

    let text_x = icon_rect.max.x + 12.0;
    let max_width = rect.max.x - text_x - 8.0;
    let title = ui.painter().layout(
        entry.title.clone(),
        egui::FontId::proportional(16.0),
        Color32::WHITE,
        max_width,
    );
    let file = ui.painter().layout(
        entry.file_name.clone(),
        egui::FontId::proportional(12.0),
        Color32::GRAY,
        max_width,
    );
    painter.galley(egui::pos2(text_x, rect.min.y + 16.0), title, Color32::WHITE);
    painter.galley(egui::pos2(text_x, rect.min.y + 42.0), file, Color32::GRAY);

    response.on_hover_text(entry.path.display().to_string())
}
