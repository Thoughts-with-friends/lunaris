//! LAN multiplayer window: host or join a melonDS LAN session (`ds_net`,
//! through `ds_net_gui::LanSession`), see its players, and the emulated
//! Wi-Fi's progress.
//!
//! ```text
//!  Host       LanSession::host        UDP 7064 (ENet), discovery on 7063
//!  Join       LanSession::join        worker thread, up to ~5 s per try
//!  every repaint  tick: join result, ENet pump, and a fresh NetLink on the
//!                 console whenever a session starts (melonDS puts a console
//!                 on the medium only when it powers its Wi-Fi on)
//! ```
//!
//! melonDS's LAN backend is process-wide and one session per process; Thread
//! Mode (the `Local` backend) and a LAN session exclude each other.

use std::time::Duration;

use ds_net::NetLink;
use ds_net_gui::{LanSession, PlayerStatus, SessionState};
use eframe::egui;
use lunaris_gui_common::{config::Config, console::Console as NDS};

/// How long the console (run on the UI thread) waits for its peers per
/// frame before carrying on without them.
const STALL_BUDGET: Duration = Duration::from_millis(100);

/// The LAN window's state and the session, made when first used (making it
/// selects melonDS's LAN backend).
#[derive(Default)]
pub struct LanRoomState {
    is_open: bool,
    host_ip_input: String,
    session: Option<LanSession>,
    /// Session generation the console's link was made in.
    linked: Option<u64>,
    last_error: Option<String>,
}

/// What the room window needs `main.rs` to do that it can't do itself.
pub enum LanUiAction {
    None,
    SaveConfig,
}

const fn status_label(status: PlayerStatus) -> &'static str {
    match status {
        PlayerStatus::Host => "host",
        PlayerStatus::Client => "guest",
        PlayerStatus::Connecting => "connecting",
        PlayerStatus::Disconnected => "left",
        PlayerStatus::None => "-",
    }
}

impl LanRoomState {
    pub fn menu_item(&mut self, ui: &mut egui::Ui) {
        if ui.checkbox(&mut self.is_open, "LAN Room").clicked() {
            ui.close();
        }
    }

    fn in_session(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.state().in_session())
    }

    /// `true` while in a session: loading a savestate would desync the other
    /// consoles, so `main.rs` refuses Load State then.
    pub fn blocks_state_load(&self) -> bool {
        self.in_session()
    }

    /// The console was rebuilt (open ROM, Reset, Import Save): it gets a
    /// fresh link at the next [`Self::show`].
    pub const fn on_rom_changed(&mut self, _nds: &NDS) {
        self.linked = None;
    }

    fn leave(&mut self, nds: &mut NDS) {
        if let Some(session) = &mut self.session {
            session.leave();
        }
        if self.linked.take().is_some() {
            nds.set_mp_link(None);
        }
    }

    /// Services the session and keeps the console's link current. Runs on
    /// every repaint, open or not.
    fn tick(&mut self, nds: &mut NDS) {
        let Some(session) = &mut self.session else {
            return;
        };
        session.tick(self.linked.is_some() && nds.has_rom());
        if let SessionState::Failed(e) = session.state() {
            self.last_error = Some(e.clone());
            session.leave();
        }
        let generation = session.generation();
        if session.state().in_session()
            && !session.busy()
            && nds.has_rom()
            && self.linked != Some(generation)
        {
            nds.set_mp_link(Some(Box::new(NetLink::new(0).with_stall_budget(Some(STALL_BUDGET)))));
            self.linked = Some(generation);
        }
    }

    /// Draws the window (if open) and returns anything `main.rs` needs to
    /// do in response.
    pub fn show(&mut self, ctx: &egui::Context, config: &mut Config, nds: &mut NDS) -> LanUiAction {
        self.tick(nds);
        let mut action = LanUiAction::None;
        if !self.is_open {
            return action;
        }
        if self.in_session() || self.session.as_ref().is_some_and(LanSession::busy) {
            // The player list and the join result arrive without input.
            ctx.request_repaint_after(Duration::from_millis(500));
        }

        let mut open = self.is_open;
        egui::Window::new("LAN Room").open(&mut open).default_width(420.0).show(ctx, |ui| {
            if self.in_session() {
                self.show_room(ui, nds);
            } else {
                self.show_lobby(ui, config, nds, &mut action);
            }
        });
        self.is_open = open;
        action
    }

    fn show_lobby(
        &mut self,
        ui: &mut egui::Ui,
        config: &mut Config,
        nds: &mut NDS,
        action: &mut LanUiAction,
    ) {
        // Seed the join field on first draw.
        if self.host_ip_input.is_empty() {
            self.host_ip_input = if config.lan.last_host_ip.is_empty() {
                lunaris_gui_common::config::DEFAULT_HOST_IP.to_owned()
            } else {
                config.lan.last_host_ip.clone()
            };
        }

        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Player name");
            changed |= ui.text_edit_singleline(&mut config.lan.player_name).changed();
        });

        let joining = self.session.as_ref().is_some_and(LanSession::busy);
        let thread_mode = ds_net::backend() == ds_net::MpBackend::Local && nds.has_mp_link();
        ui.separator();
        if thread_mode {
            ui.label("Thread Mode is running: stop it to play over LAN.");
        }
        if let Some(ip) = ds_net_gui::local_ip() {
            ui.weak(format!("This PC: {ip} (UDP 7063-7064)"));
        }
        ui.add_enabled_ui(nds.has_rom() && !joining && !thread_mode, |ui| {
            if ui.button("Create Room (Host)").clicked() {
                // The backend changes: nothing may be on the medium.
                nds.set_mp_link(None);
                self.linked = None;
                let session = self.session.get_or_insert_with(LanSession::new);
                match session.host(&config.lan.player_name) {
                    Ok(()) => self.last_error = None,
                    Err(e) => self.last_error = Some(format!("failed to host: {e}")),
                }
            }

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Host IP");
                ui.text_edit_singleline(&mut self.host_ip_input);
            });
            let label = if joining { "Joining..." } else { "Join Room (Guest)" };
            if ui.button(label).clicked() {
                config.lan.last_host_ip = self.host_ip_input.trim().to_owned();
                changed = true;
                // The join worker drives the backend meanwhile.
                nds.set_mp_link(None);
                self.linked = None;
                self.last_error = None;
                let session = self.session.get_or_insert_with(LanSession::new);
                session.join(&config.lan.player_name, &config.lan.last_host_ip);
            }
        });
        if !nds.has_rom() {
            ui.weak("Load a game first.");
        }

        if let Some(err) = &self.last_error {
            ui.colored_label(egui::Color32::from_rgb(220, 80, 80), format!("\u{26A0} {err}"));
        }

        if changed {
            *action = LanUiAction::SaveConfig;
        }
    }

    fn show_room(&mut self, ui: &mut egui::Ui, nds: &mut NDS) {
        let Some(session) = &self.session else { return };
        let host = matches!(session.state(), SessionState::Hosting);
        let address = match session.state() {
            SessionState::Joined(address) => address.clone(),
            _ => ds_net_gui::local_ip().map_or_else(|| "this PC".to_owned(), |ip| ip.to_string()),
        };
        let players = session.players();
        ui.label(format!(
            "{} \u{b7} {} player(s) \u{b7} you are {}",
            address,
            players.len(),
            if host { "the host" } else { "a guest" }
        ));

        egui::Grid::new("lan_player_grid").striped(true).show(ui, |ui| {
            ui.strong("ID");
            ui.strong("Name");
            ui.strong("Status");
            ui.strong("Address");
            ui.strong("Ping");
            ui.end_row();
            for p in players {
                ui.label(p.id.to_string());
                ui.label(format!("{}{}", p.name, if p.is_local { " (you)" } else { "" }));
                ui.label(status_label(p.status));
                ui.label(p.address.to_string());
                ui.label(format!("{} ms", p.ping_ms));
                ui.end_row();
            }
        });

        ui.separator();
        Self::show_mp_status(ui, nds);

        if ui.button("Leave Room").clicked() {
            self.leave(nds);
        }
    }

    /// Renders how far the *emulated Wi-Fi* handshake got, underneath the
    /// session's player list: a game can sit in a healthy session and still
    /// never see its opponent, because local play additionally requires the
    /// emulated DS Wi-Fi to complete beacon -> association -> CMD/reply/ack.
    /// See `ds_peripherals::wifi::diag`.
    fn show_mp_status(ui: &mut egui::Ui, nds: &NDS) {
        let d = nds.wifi_diag_snapshot();
        let drops = d.drops;

        egui::CollapsingHeader::new("Wi-Fi link status").default_open(true).show(ui, |ui| {
            let ok = egui::Color32::from_rgb(110, 190, 110);
            let pending = egui::Color32::from_rgb(220, 140, 80);
            ui.weak(format!(
                "MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                d.mac[0], d.mac[1], d.mac[2], d.mac[3], d.mac[4], d.mac[5]
            ));
            let mut stage = |label: &str, reached: bool, detail: String| {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        if reached { ok } else { pending },
                        if reached { "OK" } else { "--" },
                    );
                    ui.label(label);
                    ui.weak(detail);
                });
            };

            stage(
                "1. RF channel",
                d.channel != 0,
                format!(
                    "channel {} \u{b7} chip type {} \u{b7} rf transfers {} \u{b7} changes {}",
                    d.channel, d.rf_version, d.rf_transfers, d.channel_changes
                ),
            );
            stage(
                "2. Driver setup",
                d.mode_reset > 0 || d.rxbuf_cfg > 0,
                format!("mode_reset {} / rxbuf_cfg {}", d.mode_reset, d.rxbuf_cfg),
            );
            stage(
                "3. Transmitting",
                d.loc_tx + d.beacon_tx + d.cmd_tx + d.reply_tx + d.blank_reply_tx > 0,
                format!(
                    "loc {} / beacon {} / cmd {} / reply {} / blank {} / ack {} / untuned {}",
                    d.loc_tx,
                    d.beacon_tx,
                    d.cmd_tx,
                    d.reply_tx,
                    d.blank_reply_tx,
                    d.ack_tx,
                    d.tx_dropped_no_channel
                ),
            );
            stage(
                "4. Receiving",
                d.rx_accepted > 0,
                format!("accepted {} / polls {} / empty {}", d.rx_accepted, d.rx_polls, d.rx_empty),
            );
            stage(
                "5. Classified",
                d.rxflags_beacon + d.rxflags_cmd + d.rxflags_ack + d.rxflags_reply > 0,
                format!(
                    "beacon {} / mgmt {} / data {} / cmd {} / ack {} / reply {}",
                    d.rxflags_beacon,
                    d.rxflags_mgmt,
                    d.rxflags_data,
                    d.rxflags_cmd,
                    d.rxflags_ack,
                    d.rxflags_reply
                ),
            );
            stage(
                "6. Associated",
                d.is_mp,
                format!("is_mp {} / client {} / aid {}", d.is_mp, d.is_mp_client, d.aid),
            );
            stage(
                "7. Delivered",
                d.irq0_raised > 0,
                format!(
                    "rx irq {} (masked {}) / ring reads {}",
                    d.irq0_raised, d.irq0_masked, d.rx_ring_reads
                ),
            );

            if d.last_assoc_timestamp != 0 || d.last_assoc_aid != 0 {
                ui.weak(format!(
                    "last assoc-resp  aid 0x{:04X} / mac_good {} / timestamp {}",
                    d.last_assoc_aid, d.last_assoc_mac_good, d.last_assoc_timestamp
                ));
            }

            let total_drops = drops.rx_disabled
                + drops.too_short
                + drops.bad_length
                + drops.channel_mismatch
                + drops.foreign_mp
                + drops.not_for_us
                + drops.filtered;
            if total_drops > 0 {
                ui.weak(format!(
                    "dropped {total_drops}: rx_disabled {} / too_short {} / bad_length {} \
                     / channel_mismatch {} / foreign_mp {} / not_for_us {} / filtered {}",
                    drops.rx_disabled,
                    drops.too_short,
                    drops.bad_length,
                    drops.channel_mismatch,
                    drops.foreign_mp,
                    drops.not_for_us,
                    drops.filtered,
                ));
            }

            ui.add_space(2.0);
            ui.colored_label(egui::Color32::from_rgb(210, 205, 130), d.verdict());
        });
    }
}
