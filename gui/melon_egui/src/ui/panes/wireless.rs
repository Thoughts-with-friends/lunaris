//! The wireless dialog: the shared airwaves, a LAN link, and its tuning.

use super::*;

/// Everything the shared airwaves have seen, in the detail needed to compare
/// this run against lunaris's own wireless trace.
///
/// The headline is deliberately the CMD count. DS local play only starts once
/// the host begins sending CMD frames — association succeeding is *not* the same
/// thing — and "association fine, no CMD ever sent" is exactly where lunaris
/// currently stops (`docs/design/review_mp_local2.md` §4). So the one number
/// that says whether this is working is how many CMD frames went out.
pub(super) fn wireless(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.heading("LAN room");
    ui.monospace(&app.lan_room);
    app.lan_status.show(ui);
    ui.horizontal(|ui| {
        ui.label("Host bind");
        ui.text_edit_singleline(&mut app.lan_bind_address);
    });
    ui.horizontal(|ui| {
        ui.label("Guest IP");
        // Persisted on connect, so the last address typed here comes back next
        // session — see `MelonEgui::settings`.
        ui.text_edit_singleline(&mut app.lan_guest_address);
    });
    link_quality(app, ui);
    super::remote::remote_desktop(app, ui);
    vpn_tuning(app, ui);
    ui.separator();

    let counters = app.airwaves.counters();
    let connected = app.airwaves.connected();
    let live: Vec<usize> =
        connected.iter().enumerate().filter_map(|(i, on)| on.then_some(i)).collect();

    let cmds: u64 = counters.iter().map(|c| c.sent_cmd).sum();
    let replies: u64 = counters.iter().map(|c| c.sent_reply).sum();
    let acks: u64 = counters.iter().map(|c| c.sent_ack).sum();
    let generic: u64 = counters.iter().map(|c| c.sent_generic).sum();

    // -- the verdict ----------------------------------------------------
    ui.heading("Status");
    match app.guest_frames() {
        // The second console runs on a thread of its own, so this climbing is
        // what says the pair is running *concurrently* -- which is what makes
        // a wireless round's reply arrive while the host is still asking for
        // it. See `crate::guest`.
        Some(frames) => ui.label(format!("Second console: running, frame {frames}")),
        None => ui.label("No second console. System ▸ Multiplayer ▸ Launch new instance."),
    };
    if live.is_empty() {
        ui.label(
            "No console is on the air yet. A cart only joins when it opens its \
             wireless menu, so this stays empty until then.",
        );
    } else if cmds == 0 {
        ui.colored_label(
            egui::Color32::from_rgb(0xE0, 0xA0, 0x40),
            format!(
                "{} console(s) on the air, {generic} frames exchanged, but no CMD frame \
                 has been sent.",
                live.len()
            ),
        );
        ui.label(
            "Beacons and the association handshake are ordinary frames; local play only \
             begins when the host starts an MP round with a CMD. This is the exact point \
             lunaris does not get past.",
        );
    } else {
        ui.colored_label(
            egui::Color32::from_rgb(0x60, 0xC0, 0x60),
            format!("MP rounds are running: {cmds} CMD, {replies} replies, {acks} ACK."),
        );
        if replies == 0 {
            ui.colored_label(
                egui::Color32::from_rgb(0xE0, 0xA0, 0x40),
                "The host is asking but no client has answered.",
            );
        }
    }
    ui.separator();

    // -- per console ----------------------------------------------------
    ui.heading("Per console");
    egui::ScrollArea::horizontal().id_salt("mp-counters").show(ui, |ui| {
        egui::Grid::new("mp-grid").striped(true).show(ui, |ui| {
            for heading in [
                "#",
                "on air",
                "wifi clock",
                "sent pkt",
                "CMD",
                "reply",
                "ACK",
                "recv pkt",
                "recv CMD",
                "recv reply",
                "stale",
                "AID mask",
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
                ui.monospace(if connected[i] { "yes" } else { "no" });
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
    ui.label(
        "\"stale\" counts replies discarded for arriving outside the host's round, and \
         \"AID mask\" is what the last reply collection returned - a host asking and \
         getting 0000 is a host nobody answered.",
    );
    ui.separator();

    // -- the traffic log ------------------------------------------------
    ui.horizontal(|ui| {
        ui.heading("Traffic");
        if ui.button("Clear").clicked() {
            app.airwaves.clear_log();
        }
    });
    let log = app.airwaves.log();
    if log.is_empty() {
        ui.label("(nothing yet)");
        return;
    }
    // Newest last, scrolled to the bottom, so it reads like a trace.
    egui::ScrollArea::vertical().id_salt("mp-log").max_height(220.0).stick_to_bottom(true).show(
        ui,
        |ui| {
            for event in &log {
                let kind = match event.kind {
                    Kind::Reply(aid) => format!("reply aid={aid}"),
                    other => other.label().to_owned(),
                };
                ui.monospace(format!(
                    "inst {}  t={:<12} {:<12} {} bytes",
                    event.sender, event.timestamp, kind, event.len
                ));
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
    ui.separator();
    ui.heading("Link quality");
    egui::Grid::new("link-quality").striped(true).show(ui, |ui| {
        ui.label("Round trip");
        ui.monospace(format!("{:.1} ms (jitter {:.1} ms)", stats.rtt_ms, stats.jitter_ms));
        ui.end_row();

        ui.label("Reply budget");
        ui.monospace(format!("{:.0} ms", stats.budget_ms));
        ui.end_row();

        ui.label("Rounds completed");
        match stats.round_success() {
            Some(fraction) => {
                let colour = if fraction > 0.95 {
                    egui::Color32::from_rgb(0x50, 0xC0, 0x60)
                } else if fraction > 0.8 {
                    egui::Color32::from_rgb(0xE0, 0xA0, 0x40)
                } else {
                    egui::Color32::from_rgb(0xE0, 0x60, 0x50)
                };
                ui.colored_label(
                    colour,
                    format!(
                        "{:.1}%  ({} of {})",
                        fraction * 100.0,
                        stats.rounds_answered,
                        stats.rounds_answered + stats.rounds_timed_out
                    ),
                );
            }
            None => {
                ui.label("no round yet");
            }
        }
        ui.end_row();

        ui.label("Sustainable frame rate");
        ui.monospace(format!("{:.1} fps", stats.sustainable_fps));
        ui.end_row();

        ui.label("Datagrams");
        ui.monospace(format!(
            "{} sent, {} received, {} duplicates discarded",
            stats.datagrams_sent, stats.datagrams_received, stats.duplicates_dropped
        ));
        ui.end_row();

        // Frames rather than datagrams: the difference between the two is what
        // batching bought, and the difference between datagrams sent and frames
        // sent is what redundancy cost.
        ui.label("Wireless frames");
        ui.monospace(format!("{} sent, {} received", stats.frames_sent, stats.frames_received));
        ui.end_row();

        ui.label("Stale replies");
        ui.monospace(stats.stale_replies.to_string());
        ui.end_row();

        ui.label("Wireless");
        ui.label(if stats.wireless_on {
            "on (the cart has opened its wireless menu)"
        } else {
            "off (the cart has not started multiplayer yet)"
        });
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
    egui::CollapsingHeader::new("VPN tuning").default_open(false).show(ui, |ui| {
        ui.label(
            "Applies to the next LAN connection. The reply budget is measured from the \
             link itself; these only bound and shape it.",
        );
        let tuning = &mut app.lan_tuning;
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut tuning.min_budget_ms).range(1..=200));
            ui.label("Minimum reply wait (ms)");
        });
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut tuning.max_budget_ms).range(1..=1000));
            ui.label("Maximum reply wait (ms)");
        })
        .response
        .on_hover_text(
            "The worst link that will still be played over. Past this a game's own \
             timeouts give up anyway.",
        );
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut tuning.jitter_factor).range(0..=16));
            ui.label("Jitter allowance");
        })
        .response
        .on_hover_text("Multiples of the measured jitter added on top of the round trip.");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut tuning.reply_copies).range(1..=4));
            ui.label("Copies of each reply");
        })
        .response
        .on_hover_text(
            "A lost reply is a lost round, and a lost round is a communication error. \
             Sending two copies costs bandwidth and removes most single-packet losses.",
        );
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut tuning.batch_window_ms).range(0..=50));
            ui.label("Batch window (ms)");
        })
        .response
        .on_hover_text(
            "How long ordinary frames (beacons, association) may wait to share one \
             datagram. 0 sends each on its own. Rounds are never batched: a round has \
             to finish inside its own emulated frame.",
        );
        ui.checkbox(&mut tuning.pace_to_link, "Follow the link's frame rate").on_hover_text(
            "Run the console at the rate the link can sustain instead of dropping the \
             rounds it cannot service.",
        );
        if ui.button("Reset to defaults").clicked() {
            *tuning = crate::lan::Tuning::default();
        }
        app.lan_tuning.normalize();
    });
}
