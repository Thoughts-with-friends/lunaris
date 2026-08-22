//! Where saves, savestates and cheats are kept.

use super::*;

pub(super) fn paths(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.label(
        "Empty means \"beside the ROM\". By default each console keeps its own files under \
         instances/instanceN/, which is where lunaris keeps its.",
    );
    ui.separator();
    // What each row does, gathered first: the buttons need `&mut app` and the
    // labels need to read the directories, which cannot both borrow at once.
    let mut asked = None;
    let mut reset = None;
    for setting in [PathSetting::Saves, PathSetting::States] {
        let dir = match setting {
            PathSetting::Saves => app.save_dir.clone(),
            PathSetting::States => app.state_dir.clone(),
        };
        ui.horizontal(|ui| {
            ui.label(setting.label());
            let shown = dir
                .as_ref()
                .map_or_else(|| "(beside the ROM)".to_owned(), |d| d.display().to_string());
            ui.monospace(shown);
        });
        ui.horizontal(|ui| {
            if ui.button(format!("Choose {}...", setting.label().to_lowercase())).clicked() {
                asked = Some(setting);
            }
            if ui.add_enabled(dir.is_some(), egui::Button::new("Reset")).clicked() {
                reset = Some(setting);
            }
        });
        ui.separator();
    }
    if let Some(setting) = asked {
        app.ask_for_directory(setting);
    }
    if let Some(setting) = reset {
        match setting {
            PathSetting::Saves => app.save_dir = None,
            PathSetting::States => app.state_dir = None,
        }
    }

    ui.separator();
    ui.heading("Per-instance directories");
    ui.label(
        "Each console gets saves/, states/, cheats/ and its own settings.json. \
         Instance 2 is the console System ▸ Multiplayer ▸ Launch new instance opens.",
    );
    egui::Grid::new("instance-paths").striped(true).show(ui, |ui| {
        ui.label("");
        for kind in ["saves", "states", "cheats"] {
            ui.label(kind);
        }
        ui.end_row();
        for instance in 1..=config::INSTANCE_COUNT {
            ui.label(format!("instance{instance}"));
            for kind in ["saves", "states", "cheats"] {
                ui.monospace(config::instance_data_dir(instance, kind).display().to_string());
            }
            ui.end_row();
        }
    });
    if ui.button("Open the instances folder").clicked() {
        app.reveal(&config::instances_dir());
    }
    ui.separator();
    ui.label("These take effect for the next cart loaded.");
}

/// Which directory a folder dialog was opened for.
///
/// The dialog is answered several repaints after the button was clicked, so
/// "which box did this belong to" has to be carried along with it rather than
/// inferred from what happens to be on screen when the answer arrives.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum PathSetting {
    Saves,
    States,
}

impl PathSetting {
    /// Point the setting at `dir`.
    pub fn set(self, app: &mut MelonEgui, dir: std::path::PathBuf) {
        match self {
            Self::Saves => app.save_dir = Some(dir),
            Self::States => app.state_dir = Some(dir),
        }
    }

    /// The label the settings row is drawn under.
    const fn label(self) -> &'static str {
        match self {
            Self::Saves => "Save files",
            Self::States => "Savestates",
        }
    }
}
