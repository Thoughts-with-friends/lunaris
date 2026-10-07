//! The **Wireless status** dialog, top to bottom:
//!
//! 1. LAN room — status and the two address boxes;
//! 2. [`link_quality`] — the live LAN link's measurements (if one is up);
//! 3. Remote Desktop — its numbers and settings (`remote.rs`);
//! 4. [`vpn_tuning`] — the LAN transport's knobs;
//! 5. [`air_status`] — the verdict: is anyone on the air, are rounds running;
//! 6. [`per_console`] — counters per console;
//! 7. [`traffic`] — the rolling frame log.
//!
//! The headline number is the CMD count: DS local play only starts when the
//! host sends CMD frames, and "associated, but no CMD ever sent" is exactly
//! where lunaris's own wireless stops (`docs/design/review_mp_local2.md` §4).

use super::*;

/// Draw the whole dialog.
pub(super) fn wireless(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let tr = app.translations.get(app.language);
    ui.heading(tr.t(K::LanRoom));
    ui.monospace(&app.lan_room);
    app.lan_status.show(ui);
    ui.horizontal(|ui| {
        ui.label(tr.t(K::HostBind));
        ui.text_edit_singleline(&mut app.lan_bind_address);
    });
    ui.horizontal(|ui| {
        ui.label(tr.t(K::GuestIp));
        // Persisted on connect, so the last address typed here comes back next
        // session — see `MelonEgui::settings`.
        ui.text_edit_singleline(&mut app.lan_guest_address);
    });
    link_quality(app, ui);
    super::remote::remote_desktop(app, ui);
    vpn_tuning(app, ui);
    ui.separator();

    air_status(app, ui);
    ui.separator();
    per_console(app, ui);
    ui.separator();
    traffic(app, ui);
}

/// The headline verdict: is anyone on the air, and are MP rounds running?
fn air_status(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let counters = app.airwaves.counters();
    let connected = app.airwaves.connected();
    let live: Vec<usize> =
        connected.iter().enumerate().filter_map(|(i, on)| on.then_some(i)).collect();

    let cmds: u64 = counters.iter().map(|c| c.sent_cmd).sum();
    let replies: u64 = counters.iter().map(|c| c.sent_reply).sum();
    let acks: u64 = counters.iter().map(|c| c.sent_ack).sum();
    let generic: u64 = counters.iter().map(|c| c.sent_generic).sum();

    let tr = app.i18n();
    ui.heading(tr.t(K::Status));
    match app.guest_frames() {
        // The second console runs on a thread of its own, so this climbing is
        // what says the pair is running *concurrently* -- which is what makes
        // a wireless round's reply arrive while the host is still asking for
        // it. See `crate::guest`.
        Some(frames) => ui.label(tr.f(K::SecondConsoleRunning, &[&frames])),
        None => ui.label(tr.t(K::NoSecondConsoleHint)),
    };
    if live.is_empty() {
        ui.label(tr.t(K::NobodyOnAir));
    } else if cmds == 0 {
        ui.colored_label(
            egui::Color32::from_rgb(0xE0, 0xA0, 0x40),
            tr.f(K::OnAirNoCmd, &[&live.len(), &generic]),
        );
        ui.label(tr.t(K::OnAirNoCmdExplained));
    } else {
        ui.colored_label(
            egui::Color32::from_rgb(0x60, 0xC0, 0x60),
            tr.f(K::RoundsRunning, &[&cmds, &replies, &acks]),
        );
        if replies == 0 {
            ui.colored_label(egui::Color32::from_rgb(0xE0, 0xA0, 0x40), tr.t(K::NoClientAnswered));
        }
    }
}

/// One row of counters per console that has been on the air.
fn per_console(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let counters = app.airwaves.counters();
    let connected = app.airwaves.connected();
    let tr = app.i18n();
    ui.heading(tr.t(K::PerConsole));
    egui::ScrollArea::horizontal().id_salt("mp-counters").show(ui, |ui| {
        egui::Grid::new("mp-grid").striped(true).show(ui, |ui| {
            for heading in [
                "#",
                tr.t(K::ColOnAir),
                tr.t(K::ColWifiClock),
                tr.t(K::ColSentPkt),
                "CMD",
                tr.t(K::ColReply),
                "ACK",
                tr.t(K::ColRecvPkt),
                tr.t(K::ColRecvCmd),
                tr.t(K::ColRecvReply),
                tr.t(K::ColStale),
                tr.t(K::ColAidMask),
            ] {
                ui.strong(heading);
            }
            ui.end_row();

            for (i, c) in counters.iter().enumerate() {
                // Consoles that never joined and never sent anything are noise.
                if !connected[i] && c.sent_generic == 0 && c.recv_generic == 0 {
                    continue;
                }
                ui.monospace(i.to_string());
                ui.monospace(tr.t(if connected[i] { K::Yes } else { K::No }));
                ui.monospace(c.clock.to_string());
                ui.monospace(c.sent_generic.to_string());
                ui.monospace(c.sent_cmd.to_string());
                ui.monospace(c.sent_reply.to_string());
                ui.monospace(c.sent_ack.to_string());
                ui.monospace(c.recv_generic.to_string());
                ui.monospace(c.recv_cmd.to_string());
                ui.monospace(c.recv_reply.to_string());
                ui.monospace(c.stale_replies.to_string());
                ui.monospace(format!("{:04b}", c.last_reply_mask));
                ui.end_row();
            }
        });
    });
    ui.label(tr.t(K::PerConsoleExplained));
}

/// The rolling log of every frame sent, newest last.
fn traffic(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let tr = app.translations.get(app.language);
    ui.horizontal(|ui| {
        ui.heading(tr.t(K::Traffic));
        if ui.button(tr.t(K::Clear)).clicked() {
            app.airwaves.clear_log();
        }
    });
    let log = app.airwaves.log();
    if log.is_empty() {
        ui.label(tr.t(K::NothingYet));
        return;
    }
    // Newest last, scrolled to the bottom, so it reads like a trace.
    egui::ScrollArea::vertical().id_salt("mp-log").max_height(220.0).stick_to_bottom(true).show(
        ui,
        |ui| {
            for event in &log {
                let kind = match event.kind {
                    Kind::Reply(aid) => format!("{} aid={aid}", tr.t(K::ColReply)),
                    Kind::Generic => tr.s(K::KindPacket),
                    other => other.label().to_owned(),
                };
                // Padded here rather than in the template, which is data and
                // knows nothing of widths.
                let (stamp, kind) = (format!("{:<12}", event.timestamp), format!("{kind:<12}"));
                ui.monospace(tr.f(K::TrafficLine, &[&event.sender, &stamp, &kind, &event.len]));
            }
        },
    );
}

/// What the live LAN link is measured to be doing.
///
/// The number to read is **rounds completed**. A DS multiplayer round has to
/// finish inside one emulated frame, so a link that cannot deliver a reply in
/// time produces a communication error in the game however healthy everything
/// else looks — see [`crate::lan`].
pub(super) fn link_quality(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let Some(stats) = app.lan_stats() else {
        return;
    };
    let tr = app.i18n();
    ui.separator();
    ui.heading(tr.t(K::LinkQuality));
    egui::Grid::new("link-quality").striped(true).show(ui, |ui| {
        ui.label(tr.t(K::RoundTrip));
        let (rtt, jitter) = (format!("{:.1}", stats.rtt_ms), format!("{:.1}", stats.jitter_ms));
        ui.monospace(tr.f(K::RoundTripValue, &[&rtt, &jitter]));
        ui.end_row();

        ui.label(tr.t(K::ReplyBudget));
        ui.monospace(format!("{:.0} ms", stats.budget_ms));
        ui.end_row();

        ui.label(tr.t(K::RoundsCompleted));
        match stats.round_success() {
            Some(fraction) => {
                let colour = if fraction > 0.95 {
                    egui::Color32::from_rgb(0x50, 0xC0, 0x60)
                } else if fraction > 0.8 {
                    egui::Color32::from_rgb(0xE0, 0xA0, 0x40)
                } else {
                    egui::Color32::from_rgb(0xE0, 0x60, 0x50)
                };
                let percent = format!("{:.1}", fraction * 100.0);
                let total = stats.rounds_answered + stats.rounds_timed_out;
                ui.colored_label(
                    colour,
                    tr.f(K::RoundsValue, &[&percent, &stats.rounds_answered, &total]),
                );
            }
            None => {
                ui.label(tr.t(K::NoRoundYet));
            }
        }
        ui.end_row();

        ui.label(tr.t(K::SustainableFps));
        ui.monospace(format!("{:.1} fps", stats.sustainable_fps));
        ui.end_row();

        ui.label(tr.t(K::Datagrams));
        ui.monospace(tr.f(
            K::DatagramsValue,
            &[&stats.datagrams_sent, &stats.datagrams_received, &stats.duplicates_dropped],
        ));
        ui.end_row();

        // Frames rather than datagrams: the difference between the two is what
        // batching bought, and the difference between datagrams sent and frames
        // sent is what redundancy cost.
        ui.label(tr.t(K::WirelessFrames));
        ui.monospace(tr.f(K::SentReceived, &[&stats.frames_sent, &stats.frames_received]));
        ui.end_row();

        ui.label(tr.t(K::StaleReplies));
        ui.monospace(stats.stale_replies.to_string());
        ui.end_row();

        ui.label(tr.t(K::Wireless));
        ui.label(tr.t(if stats.wireless_on { K::WirelessOn } else { K::WirelessOff }));
        ui.end_row();
    });
}

/// The knobs behind the LAN transport's behaviour on a slow link.
///
/// Deliberately editable rather than hidden: what a VPN needs varies enormously,
/// and a value that is right for a 30 ms tunnel wastes a whole frame on a 3 ms
/// one. Changes apply to the *next* connection.
pub(super) fn vpn_tuning(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.separator();
    let tr = app.translations.get(app.language);
    egui::CollapsingHeader::new(tr.t(K::VpnTuning)).id_salt("vpn-tuning").default_open(false).show(
        ui,
        |ui| {
            ui.label(tr.t(K::VpnAppliesNext));
            let tuning = &mut app.lan_tuning;
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tuning.min_budget_ms).range(1..=200));
                ui.label(tr.t(K::MinBudget));
            });
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tuning.max_budget_ms).range(1..=1000));
                ui.label(tr.t(K::MaxBudget));
            })
            .response
            .on_hover_text(tr.t(K::MaxBudgetHint));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tuning.jitter_factor).range(0..=16));
                ui.label(tr.t(K::JitterFactor));
            })
            .response
            .on_hover_text(tr.t(K::JitterFactorHint));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tuning.reply_copies).range(1..=4));
                ui.label(tr.t(K::ReplyCopies));
            })
            .response
            .on_hover_text(tr.t(K::ReplyCopiesHint));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tuning.batch_window_ms).range(0..=50));
                ui.label(tr.t(K::BatchWindow));
            })
            .response
            .on_hover_text(tr.t(K::BatchWindowHint));
            ui.checkbox(&mut tuning.pace_to_link, tr.t(K::PaceToLink))
                .on_hover_text(tr.t(K::PaceToLinkHint));
            if ui.button(tr.t(K::ResetToDefaults)).clicked() {
                *tuning = crate::lan::Tuning::default();
            }
            app.lan_tuning.normalize();
        },
    );
}
