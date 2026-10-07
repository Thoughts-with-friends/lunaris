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
        ui.label(app.i18n().t(K::NoCartRunning));
        return;
    };

    let mut closed = lid;
    if ui.checkbox(&mut closed, app.i18n().t(K::LidClosed)).changed() {
        app.set_lid_closed(closed);
    }
    ui.label(app.i18n().t(K::LidExplained));
    ui.separator();

    let mut okay = battery;
    ui.label(app.i18n().t(K::BatteryLevel));
    let mut changed = ui.radio_value(&mut okay, true, app.i18n().t(K::BatteryOkay)).changed();
    changed |= ui.radio_value(&mut okay, false, app.i18n().t(K::BatteryLow)).changed();
    if changed {
        app.set_battery_okay(okay);
    }
    ui.label(app.i18n().t(K::BatteryExplained));
}

/// What the last stopped console left behind.
///
/// melonDS has no such dialog: it puts the reason in a message box and the
/// core's log in a terminal nobody launched it from. A console that stops
/// mid-session — which is what local wireless play has been doing — needs its
/// account of itself somewhere it can be copied out of.
pub(super) fn crash(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let tr = app.i18n();
    let Some(report) = app.crash_report.clone() else {
        ui.label(tr.t(K::NothingStopped));
        return;
    };
    ui.horizontal(|ui| {
        if ui.button(tr.t(K::Copy)).clicked() {
            ui.ctx().copy_text(report.clone());
        }
        ui.label(tr.f(K::AlsoWrittenTo, &[&config::config_dir().join("last-stop.txt").display()]));
    });
    // The report itself is a diagnostic for whoever reads the bug, and stays
    // in English whatever the UI is in.
    ui.small(tr.t(K::ReportStaysEnglish));
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
    let tr = app.translations.get(app.language);
    ui.label(tr.t(K::ClockExplained));
    ui.separator();
    let clock = &mut app.clock;
    egui::Grid::new("datetime").show(ui, |ui| {
        for (label, value, range) in [
            (K::Year, &mut clock.year, 2000..=2099),
            (K::Month, &mut clock.month, 1..=12),
            (K::Day, &mut clock.day, 1..=31),
            (K::Hour, &mut clock.hour, 0..=23),
            (K::Minute, &mut clock.minute, 0..=59),
            (K::Second, &mut clock.second, 0..=59),
        ] {
            ui.label(tr.t(label));
            ui.add(egui::DragValue::new(value).range(range));
            ui.end_row();
        }
    });
    ui.separator();
    // Copied out: the buttons call methods that need all of `app`.
    let (apply, now) = (tr.s(K::Apply), tr.s(K::NowUtc));
    ui.horizontal(|ui| {
        if ui.button(apply).clicked() {
            app.apply_clock();
        }
        if ui.button(now).clicked() {
            app.clock = crate::emu::utc_clock();
        }
    });
    app.clock_note.show(ui);
}

pub(super) fn rom_info(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let Some(info) = app.cart_info() else {
        ui.label(app.i18n().t(K::NoCartLoaded));
        return;
    };
    egui::Grid::new("rom-info").show(ui, |ui| {
        for (label, value) in info {
            ui.label(app.i18n().t(label));
            ui.label(value);
            ui.end_row();
        }
    });
}
