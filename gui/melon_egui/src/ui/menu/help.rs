//! The **Help** menu.

use egui::Ui;

use super::{Action, Picked};
use crate::{
    app::{MelonEgui, Pane},
    i18n::I18nKey as K,
};

pub(super) fn help_menu(app: &MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut m = Picked::default();
    ui.menu_button(app.i18n().s(K::HelpLabel), |ui| m.pane(app, ui, true, K::About, Pane::About));
    m.0
}
