//! The **View** menu: how the two screens are arranged and scaled.

use egui::Ui;

use super::{Action, Picked};
use crate::{
    app::MelonEgui,
    i18n::I18nKey as K,
    ui::view::{AspectRatio, Rotation, SCREEN_GAPS, ScreenLayout, ScreenSizing},
};

pub(super) fn view_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut m = Picked::default();
    // Borrowed as the one field rather than through `app.i18n()`, so the radio
    // buttons below can hold `&mut app.view` beside it.
    let tr = app.translations.get(app.language);
    ui.menu_button(tr.t(K::ViewLabel), |ui| {
        ui.menu_button(tr.t(K::ScreenSize), |ui| {
            for scale in 1..=4 {
                m.entry(ui, true, &format!("{scale}x"), Action::ScreenSize(scale as f32));
            }
        });

        let view = &mut app.view;
        ui.menu_button(tr.t(K::ScreenRotation), |ui| {
            for rotation in Rotation::ALL {
                ui.radio_value(&mut view.rotation, rotation, format!("{}°", rotation.degrees()));
            }
        });
        ui.menu_button(tr.t(K::ScreenGap), |ui| {
            for gap in SCREEN_GAPS {
                ui.radio_value(&mut view.gap, gap, format!("{gap} px"));
            }
        });
        ui.menu_button(tr.t(K::ScreenLayout), |ui| {
            for layout in ScreenLayout::ALL {
                ui.radio_value(&mut view.layout, layout, layout.label(tr));
            }
            ui.separator();
            ui.checkbox(&mut view.swap, tr.t(K::SwapScreens));
        });
        ui.menu_button(tr.t(K::ScreenSizing), |ui| {
            for sizing in ScreenSizing::ALL {
                ui.radio_value(&mut view.sizing, sizing, sizing.label(tr));
            }
            ui.separator();
            ui.checkbox(&mut view.integer_scaling, tr.t(K::IntegerScaling));
        });
        ui.menu_button(tr.t(K::AspectRatio), |ui| {
            // Per screen, and labelled per screen, exactly as melonDS lists it.
            for aspect in AspectRatio::ALL {
                ui.radio_value(
                    &mut view.aspect_top,
                    aspect,
                    format!("{} {}", tr.t(K::TopScreen), aspect.label(tr)),
                );
            }
            ui.separator();
            for aspect in AspectRatio::ALL {
                ui.radio_value(
                    &mut view.aspect_bottom,
                    aspect,
                    format!("{} {}", tr.t(K::BottomScreen), aspect.label(tr)),
                );
            }
        });
        ui.separator();

        m.entry(ui, true, tr.t(K::NewWindow), Action::NewWindow);
        ui.separator();

        let view = &mut app.view;
        ui.checkbox(&mut view.filtering, tr.t(K::ScreenFiltering));
        ui.checkbox(&mut view.show_osd, tr.t(K::ShowOsd));
    });
    m.0
}
