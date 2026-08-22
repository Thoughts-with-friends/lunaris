//! Remote Desktop: what a session is doing, and its knobs.
//!
//! Drawn inside the wireless dialog, immediately below the LAN link quality —
//! because a link quality that reads badly is exactly what sends someone
//! looking for this, and for a link past a few milliseconds no LAN tuning can
//! help. See [`crate::remote`].

use super::*;

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
    use crate::i18n::I18nKey as K;
    ui.separator();
    ui.heading(app.i18n().t(K::RemoteDesktop));
    ui.small(app.i18n().t(K::RemoteDesktopExplained));

    if let Some(stats) = app.remote_stats {
        ui.separator();
        egui::Grid::new("remote-stats").striped(true).show(ui, |ui| {
            ui.label(app.i18n().t(K::InputLatency));
            // A button press reaches the console in half a round trip and the
            // resulting picture comes back in the other half, plus the frame it
            // was drawn in. Saying so is more use than the raw round trip.
            ui.monospace(format!("{:.0} ms", stats.rtt_ms + 16.7));
            ui.end_row();

            ui.label(app.i18n().t(K::RoundTrip));
            ui.monospace(format!("{:.1} ms", stats.rtt_ms));
            ui.end_row();

            ui.label(app.i18n().t(K::Video));
            ui.monospace(format!(
                "{:.0} fps, {:.2} Mbit/s  ({} tiles, {} B in the last frame)",
                stats.video_fps,
                stats.video_megabits_per_second(),
                stats.last_frame_tiles,
                stats.last_frame_bytes,
            ));
            ui.end_row();

            // The saving is printed rather than claimed: the sound would be a
            // third of the link at the console's own rate.
            ui.label(app.i18n().t(K::StreamAudio));
            ui.monospace(format!(
                "{} Hz, {:.2} Mbit/s  (was {:.2} at 48 kHz)",
                stats.audio_rate,
                stats.audio_megabits_per_second(),
                crate::remote::RemoteStats::audio_megabits_per_second_raw(),
            ));
            ui.end_row();

            ui.label("Frames skipped");
            ui.monospace(format!(
                "{} (skipping costs smoothness, never latency)",
                stats.frames_skipped
            ));
            ui.end_row();

            ui.label("Frames");
            ui.monospace(format!(
                "{} ({} datagrams, {} MiB, {} discarded)",
                stats.frames,
                stats.video_datagrams,
                stats.video_bytes / (1024 * 1024),
                stats.discarded
            ));
            ui.end_row();

            ui.label("Session");
            ui.label(if stats.connected { "connected" } else { "not connected" });
            ui.end_row();

            ui.label("Audio delivered");
            ui.monospace(format!(
                "{} pairs, {} dropped to stay in step",
                stats.audio_pairs, stats.audio_dropped
            ));
            ui.end_row();

            ui.label("Input samples");
            ui.monospace(stats.inputs.to_string());
            ui.end_row();
        });
        ui.small(app.i18n().t(K::RemoteClientOwnsNothing));
    }

    egui::CollapsingHeader::new(app.i18n().s(K::RemoteDesktopSettings)).default_open(false).show(
        ui,
        |ui| {
            ui.label("Applies to the next Remote Desktop session.");
            let (refresh, audio, lag, port) = (
                app.i18n().s(K::RefreshPeriod),
                app.i18n().s(K::StreamAudio),
                app.i18n().s(K::AudioLagLimit),
                app.i18n().s(K::Port),
            );
            let tuning = &mut app.remote_tuning;
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tuning.refresh_period).range(1..=60));
                ui.label(refresh);
            })
            .response
            .on_hover_text(
                "Every tile is repainted at least this often, which is the whole of the \
                 loss recovery: a dropped datagram costs a few stale tiles for this many \
                 frames and nothing more. Lower recovers faster and costs bandwidth.",
            );
            ui.checkbox(&mut tuning.audio, audio);
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tuning.max_audio_lag_ms).range(20..=1000));
                ui.label(lag);
            })
            .response
            .on_hover_text(
                "Audio queued past this is dropped rather than played. Sound that is \
                 queued is sound that is late, and a queue that is never trimmed slides \
                 further behind the picture for as long as the session lasts.",
            );
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tuning.port).range(1..=65535));
                ui.label(port);
            });
            if ui.button("Reset to defaults").clicked() {
                *tuning = crate::remote::Tuning::default();
            }
            app.remote_tuning.normalize();
        },
    );
}
