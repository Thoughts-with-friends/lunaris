//! Concrete [`super::DebugWindowState`] implementations, ported 1:1 from
//! the imgui front end's `gui/src/debug/windows.rs` (same fields, same
//! `NDS` render calls) onto egui widgets.

use std::collections::VecDeque;
use std::time::Instant;

use egui_plot::{Line, Plot, PlotPoints};

use super::{DebugWindowState, Engine, GraphicsType, NDS};

/// Builds an egui combo box over one of the fixed `ENGINES`/`GRAPHICS_TYPES`
/// arrays, storing the selection as an index (mirrors the imgui port's
/// `ComboBox::build_simple` usage).
fn labeled_combo<T: Clone>(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut usize,
    options: &[T],
    label_of: impl Fn(&T) -> &str,
) {
    egui::ComboBox::from_label(label).selected_text(label_of(&options[*selected])).show_ui(
        ui,
        |ui| {
            for (i, option) in options.iter().enumerate() {
                ui.selectable_value(selected, i, label_of(option));
            }
        },
    );
}

pub struct PalettesWindowState {
    palettes_extended: bool,
    palettes_slot: u32,
    palettes_palette: u32,
    palettes_engine: usize,
    palettes_graphics_type: usize,
}

impl DebugWindowState for PalettesWindowState {
    fn new() -> Self {
        PalettesWindowState {
            palettes_extended: false,
            palettes_slot: 0,
            palettes_palette: 0,
            palettes_engine: 0,
            palettes_graphics_type: 0,
        }
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.palettes_extended, "Extended");
        if self.palettes_extended {
            if Self::GRAPHICS_TYPES[self.palettes_graphics_type] == GraphicsType::BG {
                ui.add(egui::Slider::new(&mut self.palettes_slot, 0..=3).text("Slot"));
            }
            ui.add(egui::Slider::new(&mut self.palettes_palette, 0..=15).text("Palette"));
        }

        labeled_combo(ui, "Engine", &mut self.palettes_engine, &Self::ENGINES, Engine::label);
        labeled_combo(
            ui,
            "Graphics Type",
            &mut self.palettes_graphics_type,
            &Self::GRAPHICS_TYPES,
            GraphicsType::label,
        );
    }

    fn get_pixels(&self, nds: &mut NDS) -> (Vec<u16>, usize, usize) {
        nds.render_palettes(
            self.palettes_extended,
            self.palettes_slot as usize,
            self.palettes_palette as usize,
            Self::ENGINES[self.palettes_engine],
            Self::GRAPHICS_TYPES[self.palettes_graphics_type],
        )
    }
}

pub struct MapsWindowState {
    map_engine: usize,
    map_bg_i: u32,
}

impl DebugWindowState for MapsWindowState {
    fn new() -> Self {
        MapsWindowState { map_engine: 0, map_bg_i: 0 }
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        labeled_combo(ui, "Engine", &mut self.map_engine, &Self::ENGINES, Engine::label);
        ui.add(egui::Slider::new(&mut self.map_bg_i, 0..=3).text("BG"));
    }

    fn get_pixels(&self, nds: &mut NDS) -> (Vec<u16>, usize, usize) {
        nds.render_map(Self::ENGINES[self.map_engine], self.map_bg_i as usize)
    }
}

pub struct TilesWindowState {
    tiles_engine: usize,
    tiles_graphics_type: usize,
    tiles_extended: bool,
    tiles_bitmap: bool,
    tiles_bpp8: bool,
    tiles_slot: u32,
    tiles_palette: u32,
    tiles_offset: u32,
}

impl TilesWindowState {
    const TILES_RANGES: [std::ops::RangeInclusive<u32>; 2] = [0_u32..=3, 0_u32..=1];
}

impl DebugWindowState for TilesWindowState {
    fn new() -> Self {
        TilesWindowState {
            tiles_engine: 0,
            tiles_graphics_type: 0,
            tiles_extended: false,
            tiles_bitmap: false,
            tiles_bpp8: false,
            tiles_slot: 0,
            tiles_palette: 0,
            tiles_offset: 0,
        }
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        labeled_combo(ui, "Engine", &mut self.tiles_engine, &Self::ENGINES, Engine::label);
        labeled_combo(
            ui,
            "Graphics Type",
            &mut self.tiles_graphics_type,
            &Self::GRAPHICS_TYPES,
            GraphicsType::label,
        );

        // TODO: Clean up UI - dropdown with 4 options instead of checkboxes,
        // mirroring the TODO already present in the imgui front end.
        if !self.tiles_extended && !self.tiles_bpp8 {
            ui.checkbox(&mut self.tiles_bitmap, "Bitmap");
        }

        if !self.tiles_bitmap {
            ui.checkbox(&mut self.tiles_extended, "Extended Palettes");
            if !self.tiles_extended {
                ui.checkbox(&mut self.tiles_bpp8, "256 Colors");
            } else if Self::GRAPHICS_TYPES[self.tiles_graphics_type] == GraphicsType::BG {
                ui.add(egui::Slider::new(&mut self.tiles_slot, 0..=3).text("Palette Slot"));
            }

            if self.tiles_extended || !self.tiles_bpp8 {
                ui.add(egui::Slider::new(&mut self.tiles_palette, 0..=15).text("Palette"));
            }
        }
        if Self::ENGINES[self.tiles_engine] == Engine::A {
            let range = Self::TILES_RANGES[self.tiles_graphics_type].clone();
            ui.add(egui::Slider::new(&mut self.tiles_offset, range).text("Offset"));
        }
    }

    fn get_pixels(&self, nds: &mut NDS) -> (Vec<u16>, usize, usize) {
        nds.render_tiles(
            Self::ENGINES[self.tiles_engine],
            Self::GRAPHICS_TYPES[self.tiles_graphics_type],
            self.tiles_extended,
            self.tiles_bitmap,
            self.tiles_bpp8,
            self.tiles_slot as usize,
            self.tiles_palette as usize,
            self.tiles_offset as usize,
        )
    }
}

pub struct VRAMWindowState {
    ignore_alpha: bool,
    bank: u32,
}

impl DebugWindowState for VRAMWindowState {
    fn new() -> Self {
        VRAMWindowState { ignore_alpha: false, bank: 0 }
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.ignore_alpha, "Ignore alpha");
        ui.add(egui::Slider::new(&mut self.bank, 0..=8).text("Bank"));
    }

    fn get_pixels(&self, nds: &mut NDS) -> (Vec<u16>, usize, usize) {
        nds.render_bank(self.bank as usize, self.ignore_alpha)
    }
}

/// Frame-time history window, equivalent to the imgui front end's
/// `StatsWindow`. Not a [`DebugWindowState`] (it has no pixel buffer to
/// render) so it keeps its own small open/close + plot logic, exactly as
/// the imgui version did.
pub struct StatsWindow {
    opened: bool,
    frame_times: VecDeque<f32>,
    frame_times_sum: f32,
    prev_frame_completed: Instant,
}

impl StatsWindow {
    pub const NUM_FRAME_TIMES: usize = 20 * 60;

    pub fn new() -> Self {
        StatsWindow {
            opened: false,
            frame_times: VecDeque::new(),
            frame_times_sum: 0.0,
            prev_frame_completed: Instant::now(),
        }
    }

    pub fn frame_completed(&mut self) {
        let cur_time = Instant::now();
        let frame_time = cur_time.duration_since(self.prev_frame_completed).as_secs_f32();
        self.prev_frame_completed = cur_time;
        if self.frame_times.len() == Self::NUM_FRAME_TIMES {
            self.frame_times_sum -= self.frame_times.pop_front().unwrap_or(0.0);
        }
        self.frame_times.push_back(frame_time);
        self.frame_times_sum += frame_time;
    }

    pub fn render(&mut self, ctx: &egui::Context) {
        if !self.opened {
            return;
        }
        let mut opened = self.opened;
        egui::Window::new("Performance Stats").open(&mut opened).show(ctx, |ui| {
            let points: PlotPoints =
                self.frame_times.iter().enumerate().map(|(i, &t)| [i as f64, t as f64]).collect();
            Plot::new("frame_times_plot").height(120.0).show(ui, |plot_ui| {
                plot_ui.line(Line::new("Frame Times", points));
            });

            if !self.frame_times.is_empty() {
                ui.label(format!(
                    "Average: {}",
                    self.frame_times_sum / self.frame_times.len() as f32
                ));
            }
        });
        self.opened = opened;
    }

    pub fn menu_item(&mut self, ui: &mut egui::Ui) {
        if ui.selectable_label(self.opened, "Performance Stats").clicked() {
            self.opened = !self.opened;
        }
    }
}
