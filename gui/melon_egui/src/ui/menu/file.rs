//! The **File** menu: carts, saves, savestates, and quitting.

use egui::Ui;

use super::{Action, Picked, Unavailable, unavailable};
use crate::{
    app::{MelonEgui, RECENT_LIMIT, STATE_SLOTS},
    i18n::I18nKey as K,
};

pub(super) fn file_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut m = Picked::default();
    ui.menu_button(app.i18n().s(K::FileLabel), |ui| {
        let loaded = app.is_loaded();

        m.item(app, ui, true, K::OpenRom, Action::OpenRom);
        ui.menu_button(app.i18n().s(K::OpenRecent), |ui| recent_menu(app, ui, &mut m));
        // `mds_boot` always direct-boots a cart; there is no firmware boot.
        unavailable(app, ui, K::BootFirmware, Unavailable::Bindings);
        ui.separator();

        ui.label(format!("{}: {}", app.i18n().t(K::DsSlot), app.cart_label()));
        m.item(app, ui, true, K::InsertCart, Action::InsertCart);
        m.item(app, ui, loaded, K::EjectCart, Action::EjectCart);
        ui.separator();

        // No GBA slot in the FFI: `mds_nds_new` takes one ROM.
        ui.label(format!("{}: {}", app.i18n().t(K::GbaSlot), app.i18n().t(K::None)));
        unavailable(app, ui, K::InsertRomCart, Unavailable::Bindings);
        unavailable(app, ui, K::InsertAddonCart, Unavailable::Bindings);
        unavailable(app, ui, K::EjectCart, Unavailable::Bindings);
        ui.separator();

        m.item(app, ui, loaded, K::ImportSavefile, Action::ImportSavefile);
        ui.separator();

        ui.menu_button(app.i18n().s(K::SaveState), |ui| {
            for slot in 1..=STATE_SLOTS {
                m.entry(ui, loaded, &slot.to_string(), Action::SaveState(Some(slot)));
            }
            ui.separator();
            m.item(app, ui, loaded, K::FromFile, Action::SaveState(None));
        });
        ui.menu_button(app.i18n().s(K::LoadState), |ui| {
            for slot in 1..=STATE_SLOTS {
                let exists = app.state_slot_exists(slot);
                m.entry(ui, loaded && exists, &slot.to_string(), Action::LoadState(Some(slot)));
            }
            ui.separator();
            m.item(app, ui, loaded, K::FromFile, Action::LoadState(None));
        });
        m.item(app, ui, app.can_undo_state_load(), K::UndoStateLoad, Action::UndoStateLoad);
        ui.separator();

        m.item(app, ui, true, K::OpenDirectory, Action::OpenDirectory);
        ui.separator();

        m.item(app, ui, true, K::Quit, Action::Quit);
    });
    m.0
}

/// The **Open recent** submenu: numbered like melonDS's, labelled by file
/// name, full path on hover.
fn recent_menu(app: &MelonEgui, ui: &mut Ui, m: &mut Picked) {
    let recents = app.recent_roms().to_vec();
    if recents.is_empty() {
        ui.add_enabled(false, egui::Button::new(app.i18n().s(K::NothingYet)));
    }
    for (i, path) in recents.iter().take(RECENT_LIMIT).enumerate() {
        let name = path
            .file_name()
            .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let clicked = ui
            .add(egui::Button::new(format!("{}.  {name}", i + 1)))
            .on_hover_text(path.display().to_string())
            .clicked();
        if clicked {
            ui.close();
            m.0 = Some(Action::OpenRecent(i));
        }
    }
    if !recents.is_empty() {
        ui.separator();
        m.item(app, ui, true, K::Clear, Action::ClearRecent);
    }
}
