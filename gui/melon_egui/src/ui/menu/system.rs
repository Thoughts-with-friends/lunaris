//! The **System** menu: running the console, and the links to another one.

use egui::Ui;

use super::{Action, Unavailable, item, unavailable};
use crate::{
    app::{MelonEgui, Pane},
    i18n::I18nKey as K,
};

pub(super) fn system_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    ui.menu_button(app.i18n().s(K::SystemLabel), |ui| {
        let loaded = app.is_loaded();

        // Pause is a checkbox in melonDS, and reads as one here too.
        let mut paused = app.is_paused();
        let pause_label = app.i18n().s(K::Pause);
        if ui.add_enabled(loaded, egui::Checkbox::new(&mut paused, pause_label)).clicked() {
            ui.close();
            action = Some(Action::TogglePause);
        }
        action = action.take().or_else(|| item(app, ui, loaded, K::Reset, Action::Reset));
        action = action.take().or_else(|| item(app, ui, loaded, K::Stop, Action::Stop));
        action = action.take().or_else(|| item(app, ui, loaded, K::FrameStep, Action::FrameStep));
        ui.separator();

        action = action
            .take()
            .or_else(|| item(app, ui, loaded, K::PowerManagement, Action::TogglePane(Pane::Power)));
        action = action
            .take()
            .or_else(|| item(app, ui, loaded, K::DateAndTime, Action::TogglePane(Pane::DateTime)));
        ui.separator();

        // melonDS's AR engine, running the codes from the ARM7's VBlank
        // handler exactly as the hardware does.
        let mut cheats_on = app.cheats_enabled;
        let cheats_label = app.i18n().s(K::EnableCheats);
        if ui.checkbox(&mut cheats_on, cheats_label).clicked() {
            app.cheats_enabled = cheats_on;
        }
        action = action
            .take()
            .or_else(|| item(app, ui, true, K::SetupCheats, Action::TogglePane(Pane::Cheats)));
        ui.separator();

        action = action
            .take()
            .or_else(|| item(app, ui, loaded, K::RomInfo, Action::TogglePane(Pane::RomInfo)));
        action = action
            .take()
            .or_else(|| item(app, ui, loaded, K::RamSearch, Action::TogglePane(Pane::RamSearch)));
        // No DSi mode in this build at all.
        unavailable(app, ui, K::ManageDsiTitles, Unavailable::Bindings);
        ui.separator();

        ui.menu_button(app.i18n().s(K::Multiplayer), |ui| {
            let label = if app.has_guest() { K::CloseInstance } else { K::LaunchInstance };
            action = action.take().or_else(|| item(app, ui, loaded, label, Action::LaunchInstance));
            action = action.take().or_else(|| {
                item(app, ui, true, K::WirelessStatus, Action::TogglePane(Pane::Wireless))
            });
            ui.separator();
            ui.label(app.i18n().t(K::LanRoom));
            ui.monospace(&app.lan_room);
            ui.label(format!("{}: {}", app.i18n().t(K::HostBind), app.lan_bind_address));
            ui.label(format!("{}: {}", app.i18n().t(K::GuestIp), app.lan_guest_address));
            ui.small(&app.lan_status.text);
            // The one number that says whether a link is working; see
            // `crate::lan`.
            if let Some(stats) = app.lan_stats()
                && let Some(success) = stats.round_success()
            {
                ui.small(format!(
                    "{}: {:.0}%   {}: {:.0} ms   {}: {:.0} fps",
                    app.i18n().t(K::RoundsCompleted),
                    success * 100.0,
                    app.i18n().t(K::RoundTrip),
                    stats.rtt_ms,
                    app.i18n().t(K::SustainableFps),
                    stats.sustainable_fps,
                ));
            }
            ui.separator();
            action = action
                .take()
                .or_else(|| item(app, ui, loaded, K::HostLanGame, Action::HostLanGame));
            action = action
                .take()
                .or_else(|| item(app, ui, loaded, K::GuestLanGame, Action::GuestLanGame));
        });

        // Kept as a menu of its own beside Multiplayer rather than inside it:
        // it is not another way to play the same wireless game, it is a
        // different arrangement of the machines entirely.
        ui.menu_button(app.i18n().s(K::RemoteDesktop), |ui| {
            ui.small(app.i18n().t(K::RemoteDesktopExplained));
            ui.separator();
            let running = app.remote_running();
            action = action.take().or_else(|| {
                item(app, ui, loaded && !running, K::HostRemoteDesktop, Action::HostRemoteDesktop)
            });
            action = action.take().or_else(|| {
                item(app, ui, !running, K::JoinRemoteDesktop, Action::JoinRemoteDesktop)
            });
            action = action.take().or_else(|| {
                item(app, ui, running, K::StopRemoteDesktop, Action::StopRemoteDesktop)
            });
            if let Some(stats) = app.remote_stats {
                ui.separator();
                ui.small(format!(
                    "{}: {:.0} ms   {}: {:.0} fps, {:.2} Mbit/s",
                    app.i18n().t(K::InputLatency),
                    stats.rtt_ms,
                    app.i18n().t(K::Video),
                    stats.video_fps,
                    stats.video_megabits_per_second() + stats.audio_megabits_per_second(),
                ));
            }
        });
    });
    action
}
