//! Remote Desktop: what a session is doing, and its knobs.
//!
//! Drawn inside the wireless dialog, immediately below the LAN link quality —
//! because a link quality that reads badly is exactly what sends someone
//! looking for this, and for a link past a few milliseconds no LAN tuning can
//! help. See [`crate::remote`].

use super::*;
use crate::i18n::I18nKey as K;

/// Remote Desktop mode: what it is for, what it is doing, and its knobs.
///
/// Placed above the VPN tuning deliberately. The numbers directly above this —
/// a round success rate below 100% and a sustainable frame rate below 59.83 —
/// are what send someone looking for a setting to change, and for a link past a
/// few milliseconds there **is** no setting: a synchronous round inside every
/// emulated frame caps the rate at `1/(16.7 ms + round trip)` whatever the
/// tuning says. This is the answer to that, so it belongs where the question is
/// asked.
pub(super) fn remote_desktop(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.separator();
    ui.heading(app.i18n().t(K::RemoteDesktop));
    ui.small(app.i18n().t(K::RemoteDesktopExplained));
    partner(app, ui);

    if let Some(stats) = app.remote_stats {
        session_stats(app, ui, &stats);
    }
    session_settings(app, ui);
}

/// Whether the other machine is ready: red when not, green when it is. Shown
/// whatever state this machine is in, because "is the other side up yet?" is
/// the question asked *before* pressing Host or Join.
pub(super) fn partner(app: &MelonEgui, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.strong(app.i18n().t(K::PartnerLabel));
        app.remote_readiness.show(ui);
    })
    .response
    .on_hover_text(app.i18n().t(K::PartnerExplained));
}

/// The live session's numbers: latency, video and audio bit rates, counters.
fn session_stats(app: &MelonEgui, ui: &mut egui::Ui, stats: &crate::remote::RemoteStats) {
    let tr = app.i18n();
    ui.separator();
    egui::Grid::new("remote-stats").striped(true).show(ui, |ui| {
        ui.label(tr.t(K::InputLatency));
        // A button press reaches the console in half a round trip and the
        // resulting picture comes back in the other half, plus the frame it
        // was drawn in. Saying so is more use than the raw round trip.
        ui.monospace(format!("{:.0} ms", stats.rtt_ms + 16.7));
        ui.end_row();

        ui.label(tr.t(K::RoundTrip));
        ui.monospace(format!("{:.1} ms", stats.rtt_ms));
        ui.end_row();

        ui.label(tr.t(K::Video));
        ui.monospace(tr.f(
            K::RdVideoValue,
            &[
                &format!("{:.0}", stats.video_fps),
                &format!("{:.2}", stats.video_megabits_per_second()),
                &stats.last_frame_tiles,
                &stats.last_frame_bytes,
            ],
        ));
        ui.end_row();

        // The saving is printed rather than claimed: the sound would be a
        // third of the link at the console's own rate.
        ui.label(tr.t(K::StreamAudio));
        ui.monospace(tr.f(
            K::RdAudioValue,
            &[
                &stats.audio_rate,
                &format!("{:.2}", stats.audio_megabits_per_second()),
                &format!("{:.2}", crate::remote::RemoteStats::audio_megabits_per_second_raw()),
            ],
        ));
        ui.end_row();

        ui.label(tr.t(K::RdFramesSkipped));
        ui.monospace(tr.f(K::RdFramesSkippedValue, &[&stats.frames_skipped]));
        ui.end_row();

        ui.label(tr.t(K::RdFrames));
        ui.monospace(tr.f(
            K::RdFramesValue,
            &[
                &stats.frames,
                &stats.video_datagrams,
                &(stats.video_bytes / (1024 * 1024)),
                &stats.discarded,
            ],
        ));
        ui.end_row();

        ui.label(tr.t(K::RdAudioDelivered));
        ui.monospace(tr.f(K::RdAudioDeliveredValue, &[&stats.audio_pairs, &stats.audio_dropped]));
        ui.end_row();

        ui.label(tr.t(K::RdInputSamples));
        ui.monospace(stats.inputs.to_string());
        ui.end_row();
    });
    ui.small(tr.t(K::RemoteClientOwnsNothing));
}

/// The knobs for the *next* session.
fn session_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
    // The one field, so `app.remote_tuning` can be borrowed mutably beside it.
    let tr = app.translations.get(app.language);
    egui::CollapsingHeader::new(tr.t(K::RemoteDesktopSettings))
        .id_salt("remote-desktop-settings")
        .default_open(false)
        .show(ui, |ui| {
            ui.label(tr.t(K::RdAppliesNext));
            let tuning = &mut app.remote_tuning;
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tuning.refresh_period).range(1..=60));
                ui.label(tr.t(K::RefreshPeriod));
            })
            .response
            .on_hover_text(tr.t(K::RdRefreshPeriodHint));
            ui.checkbox(&mut tuning.audio, tr.t(K::StreamAudio));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tuning.max_audio_lag_ms).range(20..=1000));
                ui.label(tr.t(K::AudioLagLimit));
            })
            .response
            .on_hover_text(tr.t(K::RdAudioLagHint));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tuning.port).range(1..=65535));
                ui.label(tr.t(K::Port));
            });
            if ui.button(tr.t(K::ResetToDefaults)).clicked() {
                *tuning = crate::remote::Tuning::default();
            }
            app.remote_tuning.normalize();
        });
}
