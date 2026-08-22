//! How the front end itself looks and reads: theme, scale, and language.

use super::*;

pub(super) fn interface(app: &mut MelonEgui, ui: &mut egui::Ui) {
    language_picker(app, ui);
    ui.separator();
    app.font_note.show(ui).on_hover_text(
        "egui's own fonts are Latin-only, so a system font is borrowed for          Japanese, Chinese and Korean. Set MELON_EGUI_FONT to a .ttf/.otf/.ttc          to choose a different one.",
    );
    ui.separator();
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

/// Choose the language the UI is drawn in.
///
/// Each language is offered under its own name, which is how a language picker
/// has to read: someone looking for Japanese is looking for 日本語, not for a
/// word they may not read. See [`crate::i18n`].
pub(super) fn language_picker(app: &mut MelonEgui, ui: &mut egui::Ui) {
    use crate::i18n::{I18nKey, Language};
    let mut chosen = app.language;
    ui.horizontal(|ui| {
        ui.label(app.i18n().t(I18nKey::LanguageLabel));
        egui::ComboBox::from_id_salt("language").selected_text(chosen.label()).show_ui(ui, |ui| {
            for language in Language::ALL {
                ui.selectable_value(&mut chosen, *language, language.label());
            }
        });
    });
    if chosen != app.language {
        app.set_language(chosen);
        app.save_settings();
    }
    if ui
        .button("Write translation templates")
        .on_hover_text(
            "Writes instances/translation.<lang>.json for every language. Edit one to              change a wording without rebuilding; it is read over the built-in text at              startup.",
        )
        .clicked()
    {
        let mut written = Vec::new();
        for language in Language::ALL {
            match crate::i18n::I18nMap::built_in(*language).save_template() {
                Ok(path) => written.push(path.display().to_string()),
                Err(error) => app.post_message(Severity::Error, format!("{error}")),
            }
        }
        if !written.is_empty() {
            app.post_message(Severity::Success, format!("wrote {}", written.join(", ")));
        }
    }
    // The built-in Japanese covers the keyed strings only; the rest of the UI
    // is still English, and saying so is better than leaving it to be noticed.
    if app.language == Language::Japanese {
        ui.small(format!(
            "{} of {} strings are keyed for translation; the rest are still English.",
            I18nKey::ALL.len() - I18nKey::UNTRANSLATED.len(),
            I18nKey::ALL.len(),
        ));
    }
}

pub(super) fn about(ui: &mut egui::Ui) {
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
