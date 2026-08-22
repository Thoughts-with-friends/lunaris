//! The **Help** menu.

use egui::Ui;

use super::{Action, item};
use crate::{
    app::{MelonEgui, Pane},
    i18n::I18nKey as K,
};

pub(super) fn help_menu(app: &MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button(app.i18n().s(K::HelpLabel), |ui| {
        action = item(app, ui, true, K::About, Action::TogglePane(Pane::About));
    });
    action
}
