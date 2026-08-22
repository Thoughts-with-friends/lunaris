//! Dialogs about the console itself: its power, its clock, its cart, and
//! why it last stopped.

use super::*;

/// melonDS's **System ▸ Power management**: the lid switch and what the
/// power-management chip says about the battery.
///
/// Both are inputs to the console rather than settings of the front end, so
/// they are read back from the core each frame instead of being mirrored here
/// — a cart that opens the lid itself is then visible in the dialog.
pub(super) fn power(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let Some((lid, battery)) = app.power_state() else {
        ui.label("No cart running.");
        return;
    };

    let mut closed = lid;
    if ui.checkbox(&mut closed, "Lid closed").changed() {
        app.set_lid_closed(closed);
    }
    ui.label("Closing the lid raises the lid IRQ, which is how a cart is told to sleep.");
    ui.separator();

    let mut okay = battery;
    ui.label("Battery level");
    let mut changed = ui.radio_value(&mut okay, true, "Okay").changed();
    changed |= ui.radio_value(&mut okay, false, "Low").changed();
    if changed {
        app.set_battery_okay(okay);
    }
    ui.label(
        "What SPI's power-management chip reports; \"Low\" is what a cart's low-battery          warning reads.",
    );
}

/// What the last stopped console left behind.
///
/// melonDS has no such dialog: it puts the reason in a message box and the
/// core's log in a terminal nobody launched it from. A console that stops
/// mid-session — which is what local wireless play has been doing — needs its
/// account of itself somewhere it can be copied out of.
pub(super) fn crash(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let Some(report) = app.crash_report.clone() else {
        ui.label("Nothing has stopped this session.");
        return;
    };
    ui.horizontal(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(report.clone());
        }
        ui.label(format!(
            "Also written to {}",
            config::config_dir().join("last-stop.txt").display()
        ));
    });
    ui.separator();
    egui::ScrollArea::both().max_height(420.0).show(ui, |ui| {
        ui.add(
            egui::TextEdit::multiline(&mut report.as_str())
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );
    });
}

pub(super) fn date_time(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.label("The DS clock is set at boot and runs on emulated time from there.");
    ui.separator();
    let clock = &mut app.clock;
    egui::Grid::new("datetime").show(ui, |ui| {
        for (label, value, range) in [
            ("Year", &mut clock.year, 2000..=2099),
            ("Month", &mut clock.month, 1..=12),
            ("Day", &mut clock.day, 1..=31),
            ("Hour", &mut clock.hour, 0..=23),
            ("Minute", &mut clock.minute, 0..=59),
            ("Second", &mut clock.second, 0..=59),
        ] {
            ui.label(label);
            ui.add(egui::DragValue::new(value).range(range));
            ui.end_row();
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            app.apply_clock();
        }
        if ui.button("Now (UTC)").clicked() {
            app.clock = crate::emu::utc_clock();
        }
    });
    ui.label(&app.clock_note);
}

pub(super) fn rom_info(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let Some(info) = app.cart_info() else {
        ui.label("no cart loaded");
        return;
    };
    egui::Grid::new("rom-info").show(ui, |ui| {
        for (label, value) in info {
            ui.label(label);
            ui.label(value);
            ui.end_row();
        }
    });
}
