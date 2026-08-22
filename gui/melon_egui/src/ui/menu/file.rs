//! The **File** menu: carts, saves, savestates, and quitting.

use egui::Ui;

use super::{Action, Unavailable, entry, item, unavailable};
use crate::{
    app::{MelonEgui, RECENT_LIMIT, STATE_SLOTS},
    i18n::I18nKey as K,
};

pub(super) fn file_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;

    ui.menu_button(app.i18n().s(K::FileLabel), |ui| {
        let loaded = app.is_loaded();

        action = action.take().or_else(|| item(app, ui, true, K::OpenRom, Action::OpenRom));
        ui.menu_button(app.i18n().s(K::OpenRecent), |ui| {
            let recents = app.recent_roms().to_vec();
            if recents.is_empty() {
                ui.add_enabled(false, egui::Button::new(app.i18n().s(K::NothingYet)));
            }
            for (i, path) in recents.iter().take(RECENT_LIMIT).enumerate() {
                // Numbered as melonDS numbers them, and labelled by file name so
                // the list stays readable with long paths.
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                let label = format!("{}.  {name}", i + 1);
                let clicked = ui
                    .add(egui::Button::new(&label))
                    .on_hover_text(path.display().to_string())
                    .clicked();
                if clicked {
                    ui.close();
                    action = Some(Action::OpenRecent(i));
                }
            }
            if !recents.is_empty() {
                ui.separator();
                action =
                    action.take().or_else(|| item(app, ui, true, K::Clear, Action::ClearRecent));
            }
        });
        // Booting the firmware needs a firmware image and a boot path the shim
        // does not offer: `mds_boot` always direct-boots a cart with FreeBIOS.
        unavailable(app, ui, K::BootFirmware, Unavailable::Bindings);
        ui.separator();

        ui.label(format!("{}: {}", app.i18n().t(K::DsSlot), app.cart_label()));
        action = action.take().or_else(|| item(app, ui, true, K::InsertCart, Action::InsertCart));
        action = action.take().or_else(|| item(app, ui, loaded, K::EjectCart, Action::EjectCart));
        ui.separator();

        // There is no GBA slot in the FFI: `mds_nds_new` takes one ROM.
        ui.label(format!("{}: {}", app.i18n().t(K::GbaSlot), app.i18n().t(K::None)));
        unavailable(app, ui, K::InsertRomCart, Unavailable::Bindings);
        unavailable(app, ui, K::InsertAddonCart, Unavailable::Bindings);
        unavailable(app, ui, K::EjectCart, Unavailable::Bindings);
        ui.separator();

        action = action
            .take()
            .or_else(|| item(app, ui, loaded, K::ImportSavefile, Action::ImportSavefile));
        ui.separator();

        ui.menu_button(app.i18n().s(K::SaveState), |ui| {
            for slot in 1..=STATE_SLOTS {
                action = action.take().or_else(|| {
                    entry(ui, loaded, &slot.to_string(), Action::SaveState(Some(slot)))
                });
            }
            ui.separator();
            action = action
                .take()
                .or_else(|| item(app, ui, loaded, K::FromFile, Action::SaveState(None)));
        });
        ui.menu_button(app.i18n().s(K::LoadState), |ui| {
            for slot in 1..=STATE_SLOTS {
                let exists = app.state_slot_exists(slot);
                action = action.take().or_else(|| {
                    entry(ui, loaded && exists, &slot.to_string(), Action::LoadState(Some(slot)))
                });
            }
            ui.separator();
            action = action
                .take()
                .or_else(|| item(app, ui, loaded, K::FromFile, Action::LoadState(None)));
        });
        action = action.take().or_else(|| {
            item(app, ui, app.can_undo_state_load(), K::UndoStateLoad, Action::UndoStateLoad)
        });
        ui.separator();

        action =
            action.take().or_else(|| item(app, ui, true, K::OpenDirectory, Action::OpenDirectory));
        ui.separator();

        action = action.take().or_else(|| item(app, ui, true, K::Quit, Action::Quit));
    });
    action
}
