//! The **View** menu: how the two screens are arranged and scaled.

use egui::Ui;

use super::{Action, entry};
use crate::{
    app::MelonEgui,
    i18n::I18nKey as K,
    ui::view::{AspectRatio, Rotation, SCREEN_GAPS, ScreenLayout, ScreenSizing},
};

pub(super) fn view_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button(app.i18n().s(K::ViewLabel), |ui| {
        ui.menu_button(app.i18n().s(K::ScreenSize), |ui| {
            for scale in 1..=4 {
                action = action.take().or_else(|| {
                    entry(ui, true, &format!("{scale}x"), Action::ScreenSize(scale as f32))
                });
            }
        });

        // The labels are read out of the map first: `app.i18n` borrows `app`,
        // and `app.view` below needs it mutably.
        let words = ViewWords::of(app);
        let view = &mut app.view;
        ui.menu_button(&words.rotation, |ui| {
            for rotation in Rotation::ALL {
                ui.radio_value(&mut view.rotation, rotation, format!("{}°", rotation.degrees()));
            }
        });
        ui.menu_button(&words.gap, |ui| {
            for gap in SCREEN_GAPS {
                ui.radio_value(&mut view.gap, gap, format!("{gap} px"));
            }
        });
        ui.menu_button(&words.layout, |ui| {
            for layout in ScreenLayout::ALL {
                ui.radio_value(&mut view.layout, layout, layout.label());
            }
            ui.separator();
            ui.checkbox(&mut view.swap, &words.swap);
        });
        ui.menu_button(&words.sizing, |ui| {
            for sizing in ScreenSizing::ALL {
                ui.radio_value(&mut view.sizing, sizing, sizing.label());
            }
            ui.separator();
            ui.checkbox(&mut view.integer_scaling, &words.integer_scaling);
        });
        ui.menu_button(&words.aspect, |ui| {
            // Per screen, and labelled per screen, exactly as melonDS lists it.
            for aspect in AspectRatio::ALL {
                ui.radio_value(
                    &mut view.aspect_top,
                    aspect,
                    format!("{} {}", words.top, aspect.label()),
                );
            }
            ui.separator();
            for aspect in AspectRatio::ALL {
                ui.radio_value(
                    &mut view.aspect_bottom,
                    aspect,
                    format!("{} {}", words.bottom, aspect.label()),
                );
            }
        });
        ui.separator();

        action = action.take().or_else(|| entry(ui, true, &words.new_window, Action::NewWindow));
        ui.separator();

        let view = &mut app.view;
        ui.checkbox(&mut view.filtering, &words.filtering);
        ui.checkbox(&mut view.show_osd, &words.show_osd);
    });
    action
}

/// The View menu's labels, taken out of the translation map before `app.view`
/// is borrowed mutably.
///
/// The borrow checker is the whole reason this exists: the menu's radio buttons
/// need `&mut app.view` for the length of the closure, and every label needs
/// `&app.i18n`. Copying eleven short strings once per menu open is cheaper than
/// the alternative and much clearer than interleaving the two.
struct ViewWords {
    rotation: String,
    gap: String,
    layout: String,
    swap: String,
    sizing: String,
    integer_scaling: String,
    aspect: String,
    top: String,
    bottom: String,
    new_window: String,
    filtering: String,
    show_osd: String,
}

impl ViewWords {
    fn of(app: &MelonEgui) -> Self {
        Self {
            rotation: app.i18n().s(K::ScreenRotation),
            gap: app.i18n().s(K::ScreenGap),
            layout: app.i18n().s(K::ScreenLayout),
            swap: app.i18n().s(K::SwapScreens),
            sizing: app.i18n().s(K::ScreenSizing),
            integer_scaling: app.i18n().s(K::IntegerScaling),
            aspect: app.i18n().s(K::AspectRatio),
            top: app.i18n().s(K::TopScreen),
            bottom: app.i18n().s(K::BottomScreen),
            new_window: app.i18n().s(K::NewWindow),
            filtering: app.i18n().s(K::ScreenFiltering),
            show_osd: app.i18n().s(K::ShowOsd),
        }
    }
}
