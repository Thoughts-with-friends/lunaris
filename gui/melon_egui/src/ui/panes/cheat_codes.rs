//! The cheat code editor.

use super::*;

/// melonDS's **System ▸ Setup cheat codes**.
///
/// The list is the cart's `.mch` — melonDS's own file, in its own format — so
/// codes written here open there and vice versa. Editing is deliberately plain:
/// a name and the `%08X %08X` lines every published code list is written in.
pub(super) fn cheat_codes(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let mut enabled = app.cheats_enabled;
    if ui
        .checkbox(&mut enabled, "Enable cheats")
        .on_hover_text(
            "Off hands the console an empty list, so the codes cost nothing at all \
             rather than merely doing nothing.",
        )
        .clicked()
    {
        app.cheats_enabled = enabled;
    }
    match app.cheat_file() {
        Some(path) => ui.label(format!("File: {}", path.display())),
        None => ui.label("No cart running; codes load with one."),
    };
    ui.separator();

    // Applied after the loop: removing an entry mid-iteration would renumber
    // the rest under the widgets already drawn.
    let mut remove = None;
    let mut edit = None;
    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        for (i, cheat) in app.cheats.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut cheat.enabled, "");
                let label = if cheat.category.is_empty() {
                    cheat.name.clone()
                } else {
                    format!("{} / {}", cheat.category, cheat.name)
                };
                let mut response = ui.label(label);
                if !cheat.description.is_empty() {
                    response = response.on_hover_text(&cheat.description);
                }
                if !cheat.is_well_formed() {
                    response.on_hover_text("This code has an odd number of words.");
                }
                if ui.small_button("Edit").clicked() {
                    edit = Some(i);
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
    });
    if let Some(i) = remove {
        app.cheats.remove(i);
    }
    if let Some(i) = edit {
        let cheat = &app.cheats[i];
        app.cheat_draft = (cheat.name.clone(), cheat.text());
        app.cheats.remove(i);
    }
    if app.cheats.is_empty() {
        ui.label("No codes. Paste one below, or read a melonDS .mch file.");
    }
    ui.separator();

    ui.heading("Add a code");
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut app.cheat_draft.0);
    });
    ui.add(
        egui::TextEdit::multiline(&mut app.cheat_draft.1)
            .hint_text("020F5CE4 000003E7")
            .desired_rows(3)
            .font(egui::TextStyle::Monospace),
    );
    ui.horizontal(|ui| {
        if ui.button("Add").clicked() {
            app.add_cheat_from_draft();
        }
        if ui.button("Clear").clicked() {
            app.cheat_draft = (String::new(), String::new());
        }
    });
    ui.separator();

    ui.horizontal(|ui| {
        if ui.add_enabled(app.cheat_file().is_some(), egui::Button::new("Save")).clicked() {
            app.save_cheats();
        }
        if ui.button("Open .mch...").clicked() {
            app.ask_for_cheat_file();
        }
    });
}
