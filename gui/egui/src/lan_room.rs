//! LAN multiplayer room window: create/join a room, view the player list
//! and link stats, and install/remove the Wi-Fi MP transport on the
//! running [`NDS`] instance. See `docs/design/design_lan.md` §11.
//!
//! Kept entirely separate from `main.rs` -- the only touch points are one
//! `mod lan_room;` declaration, one field + menu item, one call in
//! `update`, and the Load-State-blocking check, per §11.4.

use std::net::IpAddr;

use eframe::egui;
use lunaris_gui_common::config::Config;
use lunaris_net::{MpInterfaceSelector, PlayerView, Room, RoomConfig, RoomHandle};
use nds_core::nds::NDS;

enum Role {
    Host,
    Guest,
}

/// Owns the room window's UI state and (once in a room) the
/// [`RoomHandle`] used to read player list/link stats and send control
/// messages. The [`lunaris_net::NetTransport`] half of [`Room`] is handed
/// to [`MpInterfaceSelector`], which installs it on the emulator; it is
/// not kept here.
pub struct LanRoomState {
    is_open: bool,
    host_ip_input: String,
    role: Option<Role>,
    handle: Option<RoomHandle>,
    /// Which MP backend is installed on the emulator. Every
    /// install/uninstall goes through here rather than calling
    /// [`NDS::set_mp_transport`] directly, so the selected backend and the
    /// emulator's transport can never disagree.
    mp: MpInterfaceSelector,
    last_error: Option<String>,
    /// Cached so `set_mp_ready` is only sent to the room when the computed
    /// readiness actually changes, not once per repaint.
    last_ready_sent: Option<bool>,
    runahead_slider: u32,
    recv_timeout_slider: u16,
}

/// What the room window needs `main.rs` to do that it can't do itself.
pub enum LanUiAction {
    None,
    SaveConfig,
}

impl Default for LanRoomState {
    fn default() -> Self {
        LanRoomState {
            is_open: false,
            host_ip_input: String::new(),
            role: None,
            handle: None,
            mp: MpInterfaceSelector::new(),
            last_error: None,
            last_ready_sent: None,
            runahead_slider: 1000,
            recv_timeout_slider: 8,
        }
    }
}

impl LanRoomState {
    pub fn menu_item(&mut self, ui: &mut egui::Ui) {
        if ui.checkbox(&mut self.is_open, "LAN Room").clicked() {
            ui.close();
        }
    }

    /// `true` while this instance is `MpReady` (see
    /// `docs/design/design_lan.md` §13.3): loading a savestate mid-session
    /// would desync every other room member, so `main.rs` must refuse
    /// Load State in both the menu and the Shift+F5-F9 hotkey path while
    /// this holds.
    pub fn blocks_state_load(&self) -> bool {
        self.last_ready_sent == Some(true)
    }

    /// Publishes the current ROM's fingerprint to the room, if any.
    /// Call after every `create_nds`/`Reset`/`Import Save` (any point
    /// `main.rs` rebuilds `self.nds`), per
    /// `docs/design/design_lan.md` §10.3.
    pub fn on_rom_changed(&mut self, nds: &NDS) {
        if let Some(handle) = &self.handle {
            handle.set_rom_fingerprint(nds.rom_fingerprint().to_bytes());
        }
    }

    fn leave(&mut self, nds: &mut NDS) {
        if let Some(handle) = self.handle.take() {
            handle.leave();
        }
        self.role = None;
        self.last_ready_sent = None;
        // Falls back to the dummy backend, which is what removes the
        // transport from the emulator.
        self.mp.set_dummy(nds);
    }

    fn room_config(config: &Config, nds: &NDS) -> RoomConfig {
        RoomConfig {
            player_name: config.lan.player_name.clone(),
            room_name: config.lan.room_name.clone(),
            rom_fingerprint: nds.rom_fingerprint().to_bytes(),
            mac_suffix: config.lan.mac_suffix,
            max_players: config.lan.max_players,
            control_port: config.lan.control_port,
            mp_port: config.lan.mp_port,
        }
    }

    fn install_room(&mut self, room: Room, role: Role, nds: &mut NDS) {
        self.handle = Some(self.mp.set_lan(nds, room));
        self.role = Some(role);
        self.last_error = None;
    }

    /// Draws the window (if open) and returns anything `main.rs` needs to
    /// do in response. Also updates the room's MP-ready flag once per
    /// frame based on ROM-fingerprint matching (§10).
    pub fn show(&mut self, ctx: &egui::Context, config: &mut Config, nds: &mut NDS) -> LanUiAction {
        let mut action = LanUiAction::None;
        if !self.is_open {
            return action;
        }

        self.update_mp_ready(nds);

        let mut open = self.is_open;
        egui::Window::new("LAN Room").open(&mut open).default_width(420.0).show(ctx, |ui| {
            match &self.role {
                None => self.show_lobby(ui, config, nds, &mut action),
                Some(_) => self.show_room(ui, config, nds, &mut action),
            }
        });
        self.is_open = open;
        action
    }

    fn update_mp_ready(&mut self, nds: &NDS) {
        let Some(handle) = &self.handle else { return };
        let our_fp = nds.rom_fingerprint().to_bytes();
        let host_fp = handle.players().iter().find(|p| p.is_host).map(|p| p.rom_fingerprint);
        let ready = host_fp == Some(our_fp);
        if self.last_ready_sent != Some(ready) {
            handle.set_mp_ready(ready);
            self.last_ready_sent = Some(ready);
        }
    }

    fn show_lobby(
        &mut self,
        ui: &mut egui::Ui,
        config: &mut Config,
        nds: &mut NDS,
        action: &mut LanUiAction,
    ) {
        // Seed the join field on first draw. Kept here rather than in
        // `Default` because the config isn't available there, and an empty
        // field would otherwise force the user to type an address even for
        // the usual two-processes-on-one-machine case.
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

        ui.separator();
        ui.label(format!("Not in a room (MP backend: {})", self.mp.label()));

        ui.horizontal(|ui| {
            ui.label("Room name");
            changed |= ui.text_edit_singleline(&mut config.lan.room_name).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Max players");
            changed |= ui.add(egui::Slider::new(&mut config.lan.max_players, 1..=16)).changed();
        });
        if ui.button("Create Room (Host)").clicked() {
            let cfg = Self::room_config(config, nds);
            match Room::host(&cfg) {
                Ok(room) => self.install_room(room, Role::Host, nds),
                Err(e) => self.last_error = Some(format!("failed to host: {e}")),
            }
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Host IP");
            ui.text_edit_singleline(&mut self.host_ip_input);
        });
        if ui.button("Join Room (Guest)").clicked() {
            match self.host_ip_input.trim().parse::<IpAddr>() {
                Ok(ip) => {
                    config.lan.last_host_ip = self.host_ip_input.clone();
                    let cfg = Self::room_config(config, nds);
                    match Room::join(&cfg, ip) {
                        Ok(room) => self.install_room(room, Role::Guest, nds),
                        Err(e) => self.last_error = Some(format!("failed to join: {e}")),
                    }
                }
                Err(_) => self.last_error = Some("invalid IP address".to_owned()),
            }
        }

        if let Some(err) = &self.last_error {
            ui.colored_label(egui::Color32::from_rgb(220, 80, 80), format!("\u{26A0} {err}"));
        }

        if changed {
            *action = LanUiAction::SaveConfig;
        }
    }

    fn show_room(
        &mut self,
        ui: &mut egui::Ui,
        config: &mut Config,
        nds: &mut NDS,
        action: &mut LanUiAction,
    ) {
        let Some(handle) = &self.handle else { return };
        if handle.has_left() {
            self.leave(nds);
            return;
        }

        let players = handle.players();
        let self_id = handle.self_id();
        let is_host = handle.is_host();

        ui.label(format!(
            "In a room ({}, {}/{}) \u{b7} MP backend: {}",
            handle.room_name(),
            players.len(),
            config.lan.max_players,
            self.mp.label()
        ));

        egui::Grid::new("lan_player_grid").striped(true).show(ui, |ui| {
            ui.strong("ID");
            ui.strong("Name");
            ui.strong("Software");
            ui.strong("FPS");
            ui.strong("Link");
            ui.end_row();

            for p in &players {
                ui.label(format!("{}{}", p.id, if p.id == self_id { " (you)" } else { "" }));
                ui.label(format!("{}{}", p.name, if p.is_host { " \u{2605}" } else { "" }));
                ui.label(rom_label(p));
                ui.label(format!("{:.1}", p.fps_x10 as f32 / 10.0));
                ui.label(if p.mp_ready { "ready" } else { "\u{2717} diff" });
                ui.end_row();
            }
        });

        ui.separator();
        let link = handle.link_hints();
        if is_host {
            let mut link_params = handle.link_params();
            let mut changed = false;
            changed |= ui.checkbox(&mut link_params.auto, "Auto link speed").changed();
            ui.add_enabled_ui(!link_params.auto, |ui| {
                self.runahead_slider = link_params.runahead_us;
                self.recv_timeout_slider = link_params.recv_timeout_ms;
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.runahead_slider, 250..=16_000)
                            .text("Run-ahead (\u{b5}s)"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.recv_timeout_slider, 2..=40)
                            .text("Recv timeout (ms)"),
                    )
                    .changed();
                link_params.runahead_us = self.runahead_slider;
                link_params.recv_timeout_ms = self.recv_timeout_slider;
            });
            if changed {
                handle.set_link_params(link_params);
            }
        }
        ui.weak(format!(
            "runahead {}\u{b5}s \u{b7} recv timeout {}ms",
            link.runahead_us,
            link.recv_timeout.as_millis()
        ));

        // Frames the transport threw away because the emulator did not consume
        // them within one video frame, or because they overflowed the queue
        // bound. Occasional evictions are the mechanism working; a number that
        // climbs without bound means an RX backlog is poisoning the MP sync
        // clock. See `docs/design/review_mp_local2.md` P0-3.
        let evicted = handle.dropped_stale();
        if evicted > 0 {
            ui.weak(format!("stale/overflow frames dropped  {evicted}"));
        }

        ui.separator();
        Self::show_mp_status(ui, nds);

        if ui.button("Leave Room").clicked() {
            self.leave(nds);
        }

        let _ = action;
    }

    /// Renders how far the *emulated Wi-Fi* handshake got, underneath the
    /// room's own player list.
    ///
    /// The player list above only reports the `lunaris`-level connection (TCP
    /// control channel plus UDP peers). A game can sit in a perfectly healthy
    /// room and still never see its opponent, because local play additionally
    /// requires the emulated DS Wi-Fi hardware to complete beacon ->
    /// association -> CMD/reply/ack. This block shows which of those stages
    /// has actually happened, so "connected in the room but not in the game"
    /// is diagnosable without a terminal. Mirrors the `LUNARIS_MP_DIAG=1`
    /// dump; see `core/src/hw/net/wifi/diag.rs`.
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
                    "channel {} \u{b7} chip type {} \u{b7} RF[{}]=0x{:X} RF[{}]=0x{:X}{}",
                    d.channel,
                    d.rf_version,
                    d.rf_channel_index[0],
                    d.rf_regs_now[0],
                    d.rf_channel_index[1],
                    d.rf_regs_now[1],
                    if d.rf_table_empty { " \u{b7} TABLE EMPTY" } else { "" },
                ),
            );
            stage(
                "1b. RF programmed",
                d.rf_transfers > 0,
                format!(
                    "rf transfers {} (last id {} cmd {}) \u{b7} bb writes {}",
                    d.rf_transfers, d.rf_last_id, d.rf_last_cmd, d.bb_writes
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
                    "loc {} / beacon {} / cmd {} / reply {} / blank {}",
                    d.loc_tx, d.beacon_tx, d.cmd_tx, d.reply_tx, d.blank_reply_tx
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
                    "beacon {} / cmd {} / ack {} / reply {} / mgmt {}",
                    d.rxflags_beacon, d.rxflags_cmd, d.rxflags_ack, d.rxflags_reply, d.rxflags_mgmt
                ),
            );
            stage(
                "6. Associated",
                d.is_mp,
                format!("is_mp {} / client {} / aid {}", d.is_mp, d.is_mp_client, d.aid),
            );
            stage(
                "7. CMD rounds",
                d.irq12 > 0,
                format!(
                    "replies {} / empty {} / irq12 {}",
                    d.replies_answered, d.replies_empty, d.irq12
                ),
            );

            // Management-frame subtypes, named. While a link is forming this
            // is the whole story: an association response arriving (or not),
            // and a deauthentication tearing the session down.
            const MGMT_NAMES: [(usize, &str); 7] = [
                (0x0, "assoc-req"),
                (0x1, "assoc-resp"),
                (0x4, "probe-req"),
                (0x5, "probe-resp"),
                (0x8, "beacon"),
                (0xB, "auth"),
                (0xC, "deauth"),
            ];
            let mgmt: Vec<String> = MGMT_NAMES
                .iter()
                .filter(|&&(i, _)| d.rx_mgmt_subtype[i] > 0)
                .map(|&(i, name)| format!("{name} {}", d.rx_mgmt_subtype[i]))
                .collect();
            if !mgmt.is_empty() {
                ui.weak(format!("rx mgmt  {}", mgmt.join("  ")));
            }
            if d.rx_mgmt_subtype[0xB] > 0 {
                ui.weak(format!(
                    "last auth  algo {} / seq {} / status {}  \u{b7} retry-flagged rx {}",
                    d.last_auth[0], d.last_auth[1], d.last_auth[2], d.rx_retry_flagged
                ));
            }

            if d.irq13 + d.irq15 > 0 {
                ui.weak(format!(
                    "power irq  13 fired {} (slept {})  \u{b7}  15 fired {} (woke {})",
                    d.irq13, d.irq13_powered_down, d.irq15, d.irq15_woke
                ));
            }

            if d.powered_down || d.power_off_events > 0 {
                ui.weak(format!(
                    "power  down now {} \u{b7} W_ModeReset 0x{:04X} \u{b7} W_ModeWEP 0x{:04X} \u{b7} W_PowerDownCtrl 0x{:04X} \u{b7} off events {} (by mode-reset {})",
                    d.powered_down,
                    d.mode_reset_reg,
                    d.mode_wep_reg,
                    d.power_down_ctrl_reg,
                    d.power_off_events,
                    d.power_off_by_mode_reset
                ));
            }

            ui.weak(format!(
                "tx arm  W_TXSlotCmd 0x{:04X} \u{b7} W_TXReqRead 0x{:04X} \u{b7} W_RXCnt 0x{:04X} \u{b7} fire_tx {} (rx-off {})",
                d.tx_slot_cmd_reg, d.tx_req_read_reg, d.rx_cnt_reg, d.fire_tx_calls,
                d.fire_tx_rx_disabled
            ));

            if d.rx_mgmt_subtype[1] > 0 {
                ui.weak(format!(
                    "last assoc-resp  aid 0x{:04X} / mac_good {} / is_packet {} / timestamp {}",
                    d.last_assoc_aid,
                    d.last_assoc_mac_good,
                    d.last_assoc_is_packet,
                    d.last_assoc_timestamp
                ));
            }

            ui.weak(format!(
                "cmd slot  writes {} (bit15 dropped {}) / W_CmdCount writes {}",
                d.tx_slot_cmd_writes, d.tx_slot_cmd_bit15_dropped, d.cmd_count_writes
            ));

            // What the driver is polling. A Wi-Fi init that stalls spins on
            // one register; naming it turns "nothing happens" into a
            // specific unimplemented behaviour.
            let polled: Vec<String> = d
                .top_reads
                .iter()
                .filter(|&&(_, n)| n > 0)
                .map(|&(reg, n)| format!("{reg:03X}:{n}"))
                .collect();
            if !polled.is_empty() {
                ui.weak(format!("most-read regs  {}", polled.join("  ")));
            }

            let total_drops = drops.rx_disabled
                + drops.ring_unconfigured
                + drops.too_short
                + drops.bad_length
                + drops.channel_mismatch
                + drops.foreign_mp
                + drops.filtered
                + drops.ring_full
                + drops.wep_off;
            if total_drops > 0 {
                ui.weak(format!(
                    "dropped {total_drops}: rx_disabled {} / ring_unconfigured {} / too_short {} \
                     / bad_length {} / channel_mismatch {} / foreign_mp {} / filtered {} \
                     / ring_full {} / wep_off {}",
                    drops.rx_disabled,
                    drops.ring_unconfigured,
                    drops.too_short,
                    drops.bad_length,
                    drops.channel_mismatch,
                    drops.foreign_mp,
                    drops.filtered,
                    drops.ring_full,
                    drops.wep_off,
                ));
            }

            ui.add_space(2.0);
            ui.colored_label(
                egui::Color32::from_rgb(210, 205, 130),
                d.verdict(d.transport_installed),
            );
        });
    }
}

fn rom_label(p: &PlayerView) -> String {
    // The 16-byte fingerprint's first 4 bytes are the game code
    // (`RomFingerprint::game_code`, little-endian); decode it back to its
    // 4-character ASCII form for display, matching how the header itself
    // stores it. Falls back to a hex dump for an all-zero/placeholder
    // fingerprint (no ROM loaded yet).
    let code_bytes = &p.rom_fingerprint[0..4];
    if code_bytes.iter().all(|&b| b == 0) {
        return "(none)".to_owned();
    }
    match std::str::from_utf8(code_bytes) {
        Ok(s) if s.chars().all(|c| c.is_ascii_graphic()) => s.to_owned(),
        _ => format!(
            "{:02X}{:02X}{:02X}{:02X}",
            code_bytes[0], code_bytes[1], code_bytes[2], code_bytes[3]
        ),
    }
}
