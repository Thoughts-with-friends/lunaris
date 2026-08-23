//! The cheat code editor.

use super::*;

/// The right-hand editor's three boxes.
///
/// Held by the app rather than by this pane so the pane stays a pure function
/// of the state, and kept *separate* from [`crate::file::mch::Cheat`] on
/// purpose: what is typed here reaches the list only when Save is pressed. See
/// [`MelonEgui::commit_cheat_editor`] for why that matters.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct CheatEditor {
    pub name: String,
    /// The `DESC` line of the `.mch` entry — azahar's "memo".
    pub notes: String,
    /// The code as text, in the `%08X %08X` form code lists are published in.
    pub code: String,
}

/// What the Type column says for every row.
///
/// A constant rather than a field: the `.mch` format carries no per-code type,
/// and melonDS runs every code in it through the one Action Replay engine. The
/// column is kept because the layout this pane copies has it, and a column that
/// silently varied would claim a distinction the file cannot make.
const CHEAT_TYPE: &str = "Action Replay";

/// How wide the list side is, in points. The rest of the window is the editor.
const LIST_WIDTH: f32 = 300.0;

/// How tall the two halves are, in points.
///
/// A constant rather than `ui.available_height()`: this pane lives in an
/// auto-sized [`egui::Window`], where the available height is however much of
/// the screen is left, so asking for it hands each half a region hundreds of
/// points taller than its content -- which is a window with its controls
/// stranded in the middle of an empty field.
const CONTENT_HEIGHT: f32 = 340.0;

/// How long a press has to be held on a row before it becomes a drag, in
/// seconds.
///
/// Long enough that no ordinary click reaches it -- picking a code to edit must
/// never move it -- and short enough that a deliberate hold does not feel stuck.
/// Windows' own press-and-hold is around half a second; this sits just under it.
const HOLD_TO_DRAG: f64 = 0.4;

/// How narrow the editor half may get before it stops shrinking, in points.
/// Below this the three boxes are too narrow to read a code in.
const EDITOR_MIN_WIDTH: f32 = 260.0;

/// melonDS's **System ▸ Setup cheat codes**, laid out the way azahar's cheat
/// dialog is: the list of codes on the left with a checkbox and a type beside
/// each name, and the selected code's name, notes and words on the right.
///
/// The list is the cart's `.mch` — melonDS's own file, in its own format — so
/// codes written here open there and vice versa.
pub(super) fn cheat_codes(app: &mut MelonEgui, ui: &mut egui::Ui) {
    header(app, ui);
    ui.separator();
    buttons(app, ui);
    ui.separator();

    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(LIST_WIDTH, CONTENT_HEIGHT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| list(app, ui),
        );
        ui.separator();
        let rest = ui.available_width().max(EDITOR_MIN_WIDTH);
        ui.allocate_ui_with_layout(
            egui::vec2(rest, CONTENT_HEIGHT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| editor(app, ui),
        );
    });
}

/// The master switch and which file the list belongs to.
fn header(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
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
        if ui.button("Open .mch...").clicked() {
            app.ask_for_cheat_file();
        }
    });
    match app.cheat_file() {
        Some(path) => ui.label(format!("File: {}", path.display())),
        None => ui.label("No cart running; codes load with one."),
    };
}

/// Add on one line and Save/Delete on the next, both right-aligned — the
/// reference layout's arrangement, which keeps the destructive pair away from
/// the one that creates a row.
fn buttons(app: &mut MelonEgui, ui: &mut egui::Ui) {
    // Each row is wrapped in a `horizontal`: a right-to-left layout claims the
    // whole remaining rect, and in an auto-sized window that is the height of
    // the screen -- which centres the button in an empty field.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("Add cheat")
                .on_hover_text("Adds an empty, disabled code and opens it in the editor.")
                .clicked()
            {
                app.add_cheat();
            }
        });
    });
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let selected = app.cheat_selected.is_some();
            if ui
                .add_enabled(selected, egui::Button::new("Delete"))
                .on_hover_text("Removes the selected code, and writes the list to the cart's .mch.")
                .clicked()
            {
                app.delete_selected_cheat();
            }
            if ui
                .add_enabled(selected && app.cheat_file().is_some(), egui::Button::new("Save"))
                .on_hover_text(
                    "Writes the editor back into the selected code, and the whole list to \
                 the cart's .mch.",
                )
                .clicked()
            {
                app.commit_cheat_editor();
            }
        });
    });
}

/// The left half: every code, its checkbox and its type.
///
/// # Touching a row versus dragging it
///
/// A press selects the row it lands on, straight away. Only once that press has
/// been *held* for [`HOLD_TO_DRAG`] does it become a drag, and a drag dropped on
/// another row moves the code there.
///
/// The hold is what separates the two, and it has to: egui's own drag sources
/// begin dragging as soon as the pointer moves a few points while down, which on
/// a list like this means every attempt to pick a code up to edit it reorders
/// the list instead. A short touch can then never mean "select", because there
/// is no such thing as a perfectly still one.
///
/// # Reordering
///
/// Every row is also a drop target: dropping row *from* onto row *to* takes it
/// out and puts it back there, which is the order the `.mch` is then written in.
/// The order matters because Action Replay codes are run in list order and two
/// codes writing the same address do not commute.
///
/// The checkbox is outside all of this, so that enabling a code cannot start a
/// drag and a drag cannot toggle one.
fn list(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.label("Available cheats:");

    // Applied after the loop: both `select_cheat` and `move_cheat` take `app`,
    // which the rows below are still borrowing.
    let mut select = None;
    let mut reorder = None;
    let selected = app.cheat_selected;
    egui::ScrollArea::vertical().id_salt("cheat-list").max_height(CONTENT_HEIGHT - 24.0).show(
        ui,
        |ui| {
            egui::Grid::new("cheat-rows").striped(true).num_columns(3).show(ui, |ui| {
                ui.label("");
                ui.strong("Name");
                ui.strong("Type");
                ui.end_row();

                for (i, cheat) in app.cheats.iter_mut().enumerate() {
                    // The checkbox is its own hit target: enabling a code must not
                    // also drag the editor onto it, since the two are used for
                    // quite different things.
                    ui.checkbox(&mut cheat.enabled, "");
                    let label = if cheat.category.is_empty() {
                        cheat.name.clone()
                    } else {
                        format!("{} / {}", cheat.category, cheat.name)
                    };
                    // One widget with one response, rather than a label inside
                    // a drag source: two overlapping hit targets on the same
                    // rect disagree about which of them a press belongs to, and
                    // the row has to answer that itself.
                    let mut row = row_label(ui, label, selected == Some(i));
                    if !cheat.description.is_empty() {
                        row = row.on_hover_text(&cheat.description);
                    }
                    if !cheat.is_well_formed() {
                        row = row.on_hover_text("This code has an odd number of words.");
                    }
                    // Selected on the press rather than on the release, so a
                    // touch shows the code in the editor at once -- and so a
                    // press that goes on to become a drag has already put the
                    // code being dragged on screen.
                    if row.is_pointer_button_down_on() && ui.input(|i| i.pointer.any_pressed()) {
                        select = Some(i);
                    }
                    if held_long_enough(ui, &row) {
                        // Setting the payload is what starts the drag. egui
                        // clears it on release and paints the grabbing cursor
                        // meanwhile, so there is nothing to undo here.
                        egui::DragAndDrop::set_payload(ui.ctx(), i);
                        let stroke = ui.visuals().selection.stroke;
                        ui.painter().rect_stroke(row.rect, 2.0, stroke, egui::StrokeKind::Inside);
                    }
                    if let Some(from) = row.dnd_hover_payload::<usize>() {
                        insertion_marker(ui, row.rect, *from, i);
                    }
                    if let Some(from) = row.dnd_release_payload::<usize>() {
                        reorder = Some((*from, i));
                    }
                    ui.label(CHEAT_TYPE);
                    ui.end_row();
                }
            });
        },
    );
    if let Some(i) = select {
        app.select_cheat(Some(i));
    }
    // After the selection: a drop is also a click on the row it landed on, and
    // the move is what the user meant by it.
    if let Some((from, to)) = reorder {
        app.move_cheat(from, to);
    }
    if app.cheats.is_empty() {
        ui.label("No codes. Add one, or read a melonDS .mch file.");
    } else {
        ui.label(
            "Click a row to edit it. Hold it for a moment to pick it up, then drop it on \
             another row to reorder — the new order is saved as it lands.",
        );
    }
}

/// Whether the press on `row` has been held long enough to become a drag.
///
/// Measured from when the button went down anywhere, which is sound because
/// [`egui::Response::is_pointer_button_down_on`] is only true when *this* row is
/// what it went down on.
///
/// A repaint is asked for while the press is still short: a pointer held
/// perfectly still produces no events, and without this the hold would not be
/// noticed until something else woke the window up.
fn held_long_enough(ui: &egui::Ui, row: &egui::Response) -> bool {
    if !row.is_pointer_button_down_on() {
        return false;
    }
    let held = ui.input(|i| i.pointer.press_start_time().map(|start| i.time - start));
    match held {
        Some(held) if held >= HOLD_TO_DRAG => true,
        _ => {
            ui.ctx().request_repaint();
            false
        }
    }
}

/// One row of the list: a selectable label that senses dragging as well as
/// clicking.
///
/// Built by hand rather than with [`egui::Ui::selectable_label`] because that
/// one senses clicks only, and a row that is both has to be a single widget —
/// see this module's note on touching versus dragging.
fn row_label(ui: &mut egui::Ui, text: String, selected: bool) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text, font, egui::Color32::PLACEHOLDER);
    let padding = ui.spacing().button_padding;
    let (rect, response) =
        ui.allocate_at_least(galley.size() + 2.0 * padding, egui::Sense::click_and_drag());

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        if selected || response.hovered() {
            ui.painter().rect(
                rect,
                visuals.corner_radius,
                visuals.weak_bg_fill,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().galley(rect.min + padding, galley, visuals.text_color());
    }
    response
}

/// Draw the line the dragged row would land on: above the row when it is coming
/// down the list, below it when it is coming up.
///
/// Which side is not decoration — it is what the move actually does. Dropping
/// on row *to* puts the code at index *to*, and that is the row's top edge when
/// it came from above it and its bottom edge when it came from below.
fn insertion_marker(ui: &egui::Ui, row: egui::Rect, from: usize, to: usize) {
    if from == to {
        return;
    }
    let y = if from < to { row.bottom() } else { row.top() };
    let stroke = ui.visuals().widgets.active.fg_stroke;
    ui.painter().hline(row.x_range(), y, stroke);
}

/// The right half: the selected code's name, notes and words.
fn editor(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let enabled = app.cheat_selected.is_some();
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label("Name:");
            let width = ui.available_width();
            ui.add(egui::TextEdit::singleline(&mut app.cheat_editor.name).desired_width(width));
        });
        ui.add_space(4.0);

        ui.label("Notes:");
        let width = ui.available_width();
        ui.add(
            egui::TextEdit::multiline(&mut app.cheat_editor.notes)
                .desired_rows(4)
                .desired_width(width)
                .hint_text("What this code does, and where it came from."),
        );
        ui.add_space(4.0);

        ui.label("Code:");
        ui.add(
            egui::TextEdit::multiline(&mut app.cheat_editor.code)
                .desired_rows(6)
                .desired_width(width)
                .font(egui::TextStyle::Monospace)
                .hint_text("020F5CE4 000003E7"),
        );
    });
    if !enabled {
        ui.label("Select a code on the left, or press Add cheat.");
    }
}
