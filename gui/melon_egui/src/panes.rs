//! The auxiliary windows behind the menu's dialog entries.
//!
//! melonDS opens each of these as a modal Qt dialog; here they are ordinary
//! egui windows, so several can be open at once and none of them blocks
//! emulation.

use egui::Context;

use crate::{
    app::{BINDINGS, MelonEgui},
    config,
    view::AspectRatio,
};

/// One auxiliary window.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    RomInfo,
    RamSearch,
    DateTime,
    Input,
    EmuSettings,
    Preferences,
    VideoSettings,
    Interface,
    Paths,
    About,
}

impl Pane {
    /// The window title, which is also its egui identity.
    pub const fn title(self) -> &'static str {
        match self {
            Self::RomInfo => "ROM info",
            Self::RamSearch => "RAM search",
            Self::DateTime => "Date and time",
            Self::Input => "Input and hotkeys",
            Self::EmuSettings => "Emu settings",
            Self::Preferences => "Preferences",
            Self::VideoSettings => "Video settings",
            Self::Interface => "Interface settings",
            Self::Paths => "Path settings",
            Self::About => "About melon_egui",
        }
    }
}

/// Draw every open pane, closing any whose window was dismissed.
pub fn show(app: &mut MelonEgui, ctx: &Context) {
    for pane in app.open_panes() {
        let mut open = true;
        egui::Window::new(pane.title())
            .open(&mut open)
            .resizable(matches!(pane, Pane::RamSearch))
            .show(ctx, |ui| body(app, pane, ui));
        if !open {
            app.close_pane(pane);
        }
    }
}

fn body(app: &mut MelonEgui, pane: Pane, ui: &mut egui::Ui) {
    match pane {
        Pane::RomInfo => rom_info(app, ui),
        Pane::RamSearch => ram_search(app, ui),
        Pane::DateTime => date_time(app, ui),
        Pane::Input => input(ui),
        Pane::EmuSettings => emu_settings(app, ui),
        Pane::Preferences => preferences(app, ui),
        Pane::VideoSettings => video_settings(app, ui),
        Pane::Interface => interface(app, ui),
        Pane::Paths => paths(app, ui),
        Pane::About => about(ui),
    }
}

fn rom_info(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let Some(info) = app.cart_info() else {
        ui.label("no cart loaded");
        return;
    };
    egui::Grid::new("rom-info").show(ui, |ui| {
        for (label, value) in info {
            ui.label(label);
            ui.label(value);
            ui.end_row();
        }
    });
}

/// A cut-down version of melonDS's RAM search: scan main RAM for a value, then
/// narrow the surviving addresses as the value changes.
///
/// The narrowing is what makes it useful — a first scan of 4 MB finds far too
/// many addresses to read, and only repeated scans while the number on screen
/// changes isolate the one that matters.
fn ram_search(app: &mut MelonEgui, ui: &mut egui::Ui) {
    if !app.is_loaded() {
        ui.label("no cart loaded");
        return;
    }

    ui.horizontal(|ui| {
        ui.label("Value:");
        ui.text_edit_singleline(&mut app.ram_search.needle);
        egui::ComboBox::from_id_salt("ram-width")
            .selected_text(app.ram_search.width.label())
            .show_ui(ui, |ui| {
                for width in SearchWidth::ALL {
                    ui.selectable_value(&mut app.ram_search.width, width, width.label());
                }
            });
    });

    let parsed = app.ram_search.parse_needle();
    ui.horizontal(|ui| {
        if ui.add_enabled(parsed.is_some(), egui::Button::new("First scan")).clicked() {
            app.ram_first_scan();
        }
        let can_narrow = parsed.is_some() && !app.ram_search.hits.is_empty();
        if ui.add_enabled(can_narrow, egui::Button::new("Narrow")).clicked() {
            app.ram_narrow();
        }
        if ui.button("Clear").clicked() {
            app.ram_search.hits.clear();
        }
    });
    if parsed.is_none() && !app.ram_search.needle.is_empty() {
        ui.colored_label(egui::Color32::from_rgb(0xE0, 0x80, 0x60), "not a number");
    }

    ui.separator();
    ui.label(format!("{} matching addresses", app.ram_search.hits.len()));
    // Only a window's worth is listed: a first scan can match millions, and
    // nobody reads past the first screenful anyway.
    let shown: Vec<_> = app.ram_search.hits.iter().take(200).copied().collect();
    egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
        egui::Grid::new("ram-hits").striped(true).show(ui, |ui| {
            for addr in shown {
                ui.monospace(format!("{addr:08X}"));
                ui.monospace(format!("{}", app.ram_read(addr)));
                ui.end_row();
            }
        });
    });
    if app.ram_search.hits.len() > 200 {
        ui.label("(first 200 shown — narrow the search to see fewer)");
    }
}

/// How wide a value the RAM search looks for.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchWidth {
    Byte,
    Half,
    #[default]
    Word,
}

impl SearchWidth {
    pub const ALL: [Self; 3] = [Self::Byte, Self::Half, Self::Word];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Byte => "8-bit",
            Self::Half => "16-bit",
            Self::Word => "32-bit",
        }
    }

    /// Bytes per value, which is also the scan's stride: a value is only looked
    /// for where it could be aligned.
    pub const fn size(self) -> usize {
        match self {
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
        }
    }
}

/// The RAM search's state, kept between repaints.
#[derive(Default)]
pub struct RamSearch {
    pub needle: String,
    pub width: SearchWidth,
    /// Addresses still matching, narrowed by each scan.
    pub hits: Vec<u32>,
}

impl RamSearch {
    /// The value being searched for, accepting decimal or `0x`-prefixed hex.
    pub fn parse_needle(&self) -> Option<u32> {
        let text = self.needle.trim();
        let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            Some(hex) => u32::from_str_radix(hex, 16),
            None => text.parse(),
        }
        .ok()?;
        // A value too wide for the chosen width could never be found.
        let fits = match self.width {
            SearchWidth::Byte => parsed <= u32::from(u8::MAX),
            SearchWidth::Half => parsed <= u32::from(u16::MAX),
            SearchWidth::Word => true,
        };
        fits.then_some(parsed)
    }
}

fn date_time(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.label("The DS clock is set at boot and runs on emulated time from there.");
    ui.separator();
    let clock = &mut app.clock;
    egui::Grid::new("datetime").show(ui, |ui| {
        for (label, value, range) in [
            ("Year", &mut clock.year, 2000..=2099),
            ("Month", &mut clock.month, 1..=12),
            ("Day", &mut clock.day, 1..=31),
            ("Hour", &mut clock.hour, 0..=23),
            ("Minute", &mut clock.minute, 0..=59),
            ("Second", &mut clock.second, 0..=59),
        ] {
            ui.label(label);
            ui.add(egui::DragValue::new(value).range(range));
            ui.end_row();
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            app.apply_clock();
        }
        if ui.button("Now (UTC)").clicked() {
            app.clock = crate::emu::utc_clock();
        }
    });
    ui.label(&app.clock_note);
}

fn input(ui: &mut egui::Ui) {
    ui.label("Bindings are fixed in this front end.");
    ui.separator();
    egui::Grid::new("bindings").striped(true).show(ui, |ui| {
        for (key, _, name) in BINDINGS {
            ui.label(*name);
            ui.monospace(key.name());
            ui.end_row();
        }
        ui.label("Touch");
        ui.monospace("click the bottom screen");
        ui.end_row();
    });
}

fn emu_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.checkbox(&mut app.limit_framerate, "Limit framerate")
        .on_hover_text("Off runs the core as fast as it will go.");
    ui.separator();
    ui.label(app.audio_status());
    ui.separator();
    ui.label("Console: DS, direct boot, FreeBIOS + generated firmware.");
    ui.label("The shim offers no other boot mode, so there is nothing else to pick.");
    ui.separator();
    ui.checkbox(&mut app.mic_static, "Microphone: white noise")
        .on_hover_text("The only mic input this build has; carts wanting a breath hear static.");
}

fn preferences(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.checkbox(&mut app.pause_when_unfocused, "Pause when the window loses focus");
    ui.checkbox(&mut app.confirm_on_quit, "Ask before quitting with a cart running");
    ui.separator();
    ui.label("Settings are written to:");
    ui.monospace(config::config_dir().display().to_string());
}

fn video_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.label("Renderer: melonDS software rasteriser.");
    ui.label("The OpenGL renderer is excluded from this build of the core.");
    ui.separator();
    ui.checkbox(&mut app.view.filtering, "Screen filtering")
        .on_hover_text("Smooth the picture when scaled, instead of square pixels.");
    ui.separator();
    ui.label("Aspect ratio");
    egui::Grid::new("video-aspect").show(ui, |ui| {
        for (label, aspect) in
            [("Top", &mut app.view.aspect_top), ("Bottom", &mut app.view.aspect_bottom)]
        {
            ui.label(label);
            egui::ComboBox::from_id_salt(label).selected_text(aspect.label()).show_ui(ui, |ui| {
                for choice in AspectRatio::ALL {
                    ui.selectable_value(aspect, choice, choice.label());
                }
            });
            ui.end_row();
        }
    });
}

fn interface(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let mut dark = app.dark_theme;
    if ui.checkbox(&mut dark, "Dark theme").changed() {
        app.set_theme(ui.ctx(), dark);
    }
    ui.separator();
    ui.add(
        egui::Slider::new(&mut app.ui_scale, 0.75..=2.0)
            .text("UI scale")
            .custom_formatter(|value, _| format!("{value:.2}x")),
    );
    if ui.button("Apply UI scale").clicked() {
        ui.ctx().set_zoom_factor(app.ui_scale);
    }
    ui.separator();
    ui.checkbox(&mut app.view.show_osd, "Show OSD");
}

fn paths(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.label("Empty means \"beside the ROM\", which is melonDS's behaviour.");
    ui.separator();
    for (label, dir) in [("Save files", &mut app.save_dir), ("Savestates", &mut app.state_dir)] {
        ui.horizontal(|ui| {
            ui.label(label);
            let shown = dir
                .as_ref()
                .map_or_else(|| "(beside the ROM)".to_owned(), |d| d.display().to_string());
            ui.monospace(shown);
        });
        ui.horizontal(|ui| {
            if ui.button(format!("Choose {}...", label.to_lowercase())).clicked()
                && let Some(picked) = rfd::FileDialog::new().pick_folder()
            {
                *dir = Some(picked);
            }
            if ui.add_enabled(dir.is_some(), egui::Button::new("Reset")).clicked() {
                *dir = None;
            }
        });
        ui.separator();
    }
    ui.label("These take effect for the next cart loaded.");
}

fn about(ui: &mut egui::Ui) {
    ui.label("melon_egui");
    ui.label(concat!("version ", env!("CARGO_PKG_VERSION")));
    ui.separator();
    ui.label(
        "An egui front end for the melonDS core, through the melonds-rs bindings. \
         Built as a reference picture to compare lunaris against.",
    );
    ui.separator();
    ui.label("GPL-3.0-or-later, as is the melonDS core it embeds.");
}

#[cfg(test)]
mod tests {
    use super::{RamSearch, SearchWidth};

    #[test]
    fn the_needle_accepts_decimal_and_hex() {
        let mut search = RamSearch { needle: "255".into(), ..Default::default() };
        assert_eq!(search.parse_needle(), Some(255));
        search.needle = "0xFF".into();
        assert_eq!(search.parse_needle(), Some(255));
        search.needle = "  0x10  ".into();
        assert_eq!(search.parse_needle(), Some(16));
    }

    #[test]
    fn the_needle_rejects_nonsense_and_values_too_wide_for_the_width() {
        let mut search = RamSearch { needle: "abc".into(), ..Default::default() };
        assert_eq!(search.parse_needle(), None);

        search.needle = "300".into();
        search.width = SearchWidth::Byte;
        assert_eq!(search.parse_needle(), None, "300 does not fit in 8 bits");
        search.width = SearchWidth::Half;
        assert_eq!(search.parse_needle(), Some(300));
    }
}
