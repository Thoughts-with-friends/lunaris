//! How the front end itself looks and reads: theme, scale, and language.

use super::*;

pub(super) fn interface(app: &mut MelonEgui, ui: &mut egui::Ui) {
    language_picker(app, ui);
    ui.separator();
    let tr = app.translations.get(app.language);
    app.font_note().show(ui).on_hover_text(tr.t(K::FontHint));
    ui.separator();
    let mut dark = app.dark_theme;
    if ui.checkbox(&mut dark, tr.t(K::DarkTheme)).changed() {
        app.set_theme(ui.ctx(), dark);
    }
    let tr = app.translations.get(app.language);
    ui.separator();
    ui.add(
        egui::Slider::new(&mut app.ui_scale, 0.75..=2.0)
            .text(tr.t(K::UiScale))
            .custom_formatter(|value, _| format!("{value:.2}x")),
    );
    if ui.button(tr.t(K::ApplyUiScale)).clicked() {
        ui.ctx().set_zoom_factor(app.ui_scale);
    }
    ui.separator();
    ui.checkbox(&mut app.view.show_osd, tr.t(K::ShowOsd));
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
        app.retranslate_idle_status();
        app.save_settings();
    }
    if ui
        .button(app.i18n().t(I18nKey::WriteTemplates))
        .on_hover_text(app.i18n().t(I18nKey::WriteTemplatesHint))
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
            let said = app.i18n().f(I18nKey::Wrote, &[&written.join(", ")]);
            app.post_message(Severity::Success, said);
        }
    }
    // A key with no Japanese falls back to its English; saying how many is
    // better than leaving a half-translated screen to be noticed.
    if app.language == Language::Japanese && !I18nKey::UNTRANSLATED.is_empty() {
        let (keyed, all) = (I18nKey::ALL.len() - I18nKey::UNTRANSLATED.len(), I18nKey::ALL.len());
        ui.small(app.i18n().f(I18nKey::TranslationCoverage, &[&keyed, &all]));
    }
}

pub(super) fn about(app: &MelonEgui, ui: &mut egui::Ui) {
    let tr = app.i18n();
    ui.label("melon_egui");
    ui.label(tr.f(K::Version, &[&env!("CARGO_PKG_VERSION")]));
    ui.separator();
    ui.label(tr.t(K::AboutText));
    ui.separator();
    ui.label(tr.t(K::License));
}
