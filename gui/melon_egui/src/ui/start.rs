//! The start screen shown while no cart is loaded: recent ROMs and the ROM
//! folder's games as cards (`lunaris_rom_library::egui_launcher`).

use lunaris_rom_library::egui_launcher::{LauncherAction, LauncherText};

use crate::{app::*, i18n::I18nKey as K};

impl MelonEgui {
    /// Draw the start screen. A clicked game boots right away; "Open ROM..."
    /// is returned as an [`Action`] so it goes through the usual dialog.
    pub(crate) fn start_screen(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        if std::mem::take(&mut self.launcher_dirty) {
            self.launcher.rescan(&self.recents, self.rom_dir.as_deref());
        }
        let i18n = self.i18n();
        let text = LauncherText {
            title: "Lunaris DS".into(),
            subtitle: i18n.s(K::StartSubtitle),
            open_rom: format!("{}    Ctrl+O", i18n.t(K::OpenRom)),
            rom_folder: i18n.s(K::StartRomFolder),
            section: i18n.s(K::StartSection),
            empty: i18n.s(K::StartEmpty),
        };
        let picked = ui
            .scope(|ui| {
                ui.visuals_mut().override_text_color = None;
                self.launcher.show(ui, &text)
            })
            .inner?;
        match picked {
            LauncherAction::Open(rom) => {
                self.load(&rom);
                None
            }
            LauncherAction::OpenDialog => Some(Action::OpenRom),
            LauncherAction::PickFolder => {
                let mut dialog = rfd::FileDialog::new().set_title(self.i18n().s(K::PickRomFolder));
                if let Some(dir) = &self.rom_dir {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.rom_dir = Some(dir);
                    self.launcher_dirty = true;
                    self.persist();
                }
                None
            }
        }
    }
}
