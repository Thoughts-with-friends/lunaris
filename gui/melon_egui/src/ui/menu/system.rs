//! The **System** menu: running the console, and the links to another one.

use egui::Ui;

use super::{Action, Picked, Unavailable, unavailable};
use crate::{
    app::{MelonEgui, Pane},
    i18n::I18nKey as K,
};

pub(super) fn system_menu(app: &mut MelonEgui, ui: &mut Ui) -> Option<Action> {
    let mut m = Picked::default();
    ui.menu_button(app.i18n().s(K::SystemLabel), |ui| {
        let loaded = app.is_loaded();

        // Pause is a checkbox in melonDS, and reads as one here too.
        let mut paused = app.is_paused();
        let pause_label = app.i18n().s(K::Pause);
        if ui.add_enabled(loaded, egui::Checkbox::new(&mut paused, pause_label)).clicked() {
            ui.close();
            m.0 = Some(Action::TogglePause);
        }
        m.item(app, ui, loaded, K::Reset, Action::Reset);
        m.item(app, ui, loaded, K::Stop, Action::Stop);
        m.item(app, ui, loaded, K::FrameStep, Action::FrameStep);
        ui.separator();

        m.pane(app, ui, loaded, K::PowerManagement, Pane::Power);
        m.pane(app, ui, loaded, K::DateAndTime, Pane::DateTime);
        ui.separator();

        // melonDS's Action Replay engine (runs from the ARM7's VBlank IRQ).
        let mut cheats_on = app.cheats_enabled;
        let cheats_label = app.i18n().s(K::EnableCheats);
        if ui.checkbox(&mut cheats_on, cheats_label).clicked() {
            app.cheats_enabled = cheats_on;
        }
        m.pane(app, ui, true, K::SetupCheats, Pane::Cheats);
        ui.separator();

        m.pane(app, ui, loaded, K::RomInfo, Pane::RomInfo);
        m.pane(app, ui, loaded, K::RamSearch, Pane::RamSearch);
        unavailable(app, ui, K::ManageDsiTitles, Unavailable::Bindings);
        ui.separator();

        ui.menu_button(app.i18n().s(K::Multiplayer), |ui| multiplayer_menu(app, ui, &mut m));
        // A menu of its own rather than inside Multiplayer: it is a different
        // arrangement of the machines, not another way to carry the wireless.
        ui.menu_button(app.i18n().s(K::RemoteDesktop), |ui| remote_menu(app, ui, &mut m));
    });
    m.0
}

/// **Multiplayer**: the second console, and LAN play with its live status.
fn multiplayer_menu(app: &MelonEgui, ui: &mut Ui, m: &mut Picked) {
    let loaded = app.is_loaded();
    let label = if app.has_guest() { K::CloseInstance } else { K::LaunchInstance };
    m.item(app, ui, loaded, label, Action::LaunchInstance);
    m.pane(app, ui, true, K::WirelessStatus, Pane::Wireless);
    ui.separator();
    ui.label(app.i18n().t(K::LanRoom));
    ui.monospace(&app.lan_room);
    ui.label(format!("{}: {}", app.i18n().t(K::HostBind), app.lan_bind_address));
    ui.label(format!("{}: {}", app.i18n().t(K::GuestIp), app.lan_guest_address));
    ui.small(&app.lan_status.text);
    // The one number that says whether a link works; see `crate::lan`.
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
    m.item(app, ui, loaded, K::HostLanGame, Action::HostLanGame);
    m.item(app, ui, loaded, K::GuestLanGame, Action::GuestLanGame);
}

/// **Remote Desktop**: host, join or stop, with the live session's numbers.
fn remote_menu(app: &MelonEgui, ui: &mut Ui, m: &mut Picked) {
    ui.small(app.i18n().t(K::RemoteDesktopExplained));
    ui.separator();
    let (loaded, running) = (app.is_loaded(), app.remote_running());
    m.item(app, ui, loaded && !running, K::HostRemoteDesktop, Action::HostRemoteDesktop);
    m.item(app, ui, !running, K::JoinRemoteDesktop, Action::JoinRemoteDesktop);
    m.item(app, ui, running, K::StopRemoteDesktop, Action::StopRemoteDesktop);
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
}
