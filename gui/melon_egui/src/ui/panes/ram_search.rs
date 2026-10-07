//! Searching main RAM for a value, and narrowing the result.

use super::*;

/// A cut-down version of melonDS's RAM search: scan main RAM for a value, then
/// narrow the surviving addresses as the value changes.
///
/// The narrowing is what makes it useful — a first scan of 4 MB finds far too
/// many addresses to read, and only repeated scans while the number on screen
/// changes isolate the one that matters.
pub(super) fn ram_search(app: &mut MelonEgui, ui: &mut egui::Ui) {
    if !app.is_loaded() {
        ui.label(app.i18n().t(K::NoCartLoaded));
        return;
    }

    let tr = app.translations.get(app.language);
    ui.horizontal(|ui| {
        ui.label(tr.t(K::SearchValue));
        ui.text_edit_singleline(&mut app.ram_search.needle);
        egui::ComboBox::from_id_salt("ram-width")
            .selected_text(app.ram_search.width.label(tr))
            .show_ui(ui, |ui| {
                for width in SearchWidth::ALL {
                    ui.selectable_value(&mut app.ram_search.width, width, width.label(tr));
                }
            });
    });

    let parsed = app.ram_search.parse_needle();
    // Copied out: the scans need all of `app`.
    let (first, narrow, clear) = (tr.s(K::FirstScan), tr.s(K::Narrow), tr.s(K::Clear));
    ui.horizontal(|ui| {
        if ui.add_enabled(parsed.is_some(), egui::Button::new(first)).clicked() {
            app.ram_first_scan();
        }
        let can_narrow = parsed.is_some() && !app.ram_search.hits.is_empty();
        if ui.add_enabled(can_narrow, egui::Button::new(narrow)).clicked() {
            app.ram_narrow();
        }
        if ui.button(clear).clicked() {
            app.ram_search.hits.clear();
        }
    });
    let tr = app.translations.get(app.language);
    if parsed.is_none() && !app.ram_search.needle.is_empty() {
        ui.colored_label(egui::Color32::from_rgb(0xE0, 0x80, 0x60), tr.t(K::NotANumber));
    }

    ui.separator();
    ui.label(tr.f(K::MatchingAddresses, &[&app.ram_search.hits.len()]));
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
        ui.label(app.i18n().t(K::First200Shown));
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

    /// What the width box calls it, in the UI's language.
    pub fn label(self, tr: &crate::i18n::I18nMap) -> &str {
        tr.t(match self {
            Self::Byte => K::Bits8,
            Self::Half => K::Bits16,
            Self::Word => K::Bits32,
        })
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
