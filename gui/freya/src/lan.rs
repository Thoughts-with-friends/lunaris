//! LAN play (System > Multiplayer > Host / join LAN game): the egui GUI's
//! LAN room on `lunaris_net`, for the Lunaris core.
//!
//! ```text
//!  Host: Room::host(cfg)        Join: Room::join(cfg, ip)  (own thread)
//!     Room { handle, transport }
//!       transport --Command::SetLink--> emulator thread -> NDS::set_mp_transport
//!       handle    kept in Frontend::lan: players, link speed, leave
//!  every 0.5 s (input loop): room_tick -> MP-ready = same ROM as the host,
//!                            left by the host -> link removed
//!  stop / reset / another game: the link goes with the core, the room is left
//! ```
//!
//! Two windows on one PC play together over `127.0.0.1` (Launch new
//! instance, then Host in one and Join in the other). Other cores have no
//! LAN play here: [`REASON`] says why and the menu entries stay disabled.
use std::sync::Mutex;

#[cfg(any(feature = "lunaris", feature = "melon"))]
use ds_common::frontend::lock;
use freya::prelude::*;

use crate::{dialogs::note, frontend::Frontend};
#[cfg(any(feature = "lunaris", feature = "melon"))]
use crate::{
    dialogs::{field, section},
    state::Ctx,
    theme::{self, DANGER, SUCCESS, TEXT_DIM, WARNING},
};

/// Why this build has no LAN play (`None`: it has).
#[cfg(any(feature = "lunaris", feature = "melon"))]
pub const REASON: Option<&str> = None;
#[cfg(feature = "cogi")]
pub const REASON: Option<&str> =
    Some("LAN play needs the Lunaris or melonDS core (build with --backend lunaris or melon)");

/// melonDS: melon_egui's two-console UDP link (`melon_lan`).
#[cfg(feature = "melon")]
#[path = "lan_melon.rs"]
mod melon;
#[cfg(feature = "lunaris")]
pub use imp::*;
#[cfg(feature = "melon")]
pub use melon::*;

#[cfg(feature = "lunaris")]
mod imp {
    use std::{
        net::IpAddr,
        path::PathBuf,
        sync::{Arc, mpsc},
    };

    use lunaris_gui_common::config::LanConfig;
    use lunaris_net::{LinkParams, PlayerView, Room, RoomConfig, RoomHandle};

    use super::*;
    use crate::frontend::Command;

    /// The room this instance is in.
    pub struct InRoom {
        pub handle: RoomHandle,
        /// MP-ready last told to the room.
        ready: Option<bool>,
    }

    /// LAN state of the frontend.
    #[derive(Default)]
    pub struct Lan {
        pub room: Option<InRoom>,
        /// A join still connecting.
        joining: bool,
        pub error: Option<String>,
    }

    /// What the dialog shows of the room.
    #[derive(Debug, Clone, PartialEq)]
    pub struct RoomView {
        pub name: String,
        pub host: bool,
        pub self_id: u8,
        pub players: Vec<(u8, String, bool, bool, String, f32)>,
        pub link: (bool, u32, u16),
        pub dropped: u32,
    }

    fn config_path(front: &Frontend) -> PathBuf {
        front.instance.root().join("freya_lan.json")
    }

    impl Frontend {
        /// The LAN settings (`freya_lan.json`; defaults pick a random MAC
        /// suffix, written on first use so it stays).
        pub fn lan_config(&self) -> LanConfig {
            std::fs::read_to_string(config_path(self))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_else(|| {
                    let config = LanConfig::default();
                    self.set_lan_config(&config);
                    config
                })
        }

        pub fn set_lan_config(&self, config: &LanConfig) {
            if let Ok(text) = serde_json::to_string_pretty(config) {
                let _ = std::fs::write(config_path(self), text);
            }
        }

        fn room_config(&self, config: &LanConfig) -> Result<RoomConfig, String> {
            let fingerprint = self.shared.fingerprint().ok_or("Start a game first")?;
            Ok(RoomConfig {
                player_name: config.player_name.clone(),
                room_name: config.room_name.clone(),
                rom_fingerprint: fingerprint,
                mac_suffix: config.mac_suffix,
                max_players: config.max_players,
                control_port: config.control_port,
                mp_port: config.mp_port,
            })
        }

        /// Hands the room's transport to the running core.
        fn enter(&self, room: Room) -> Result<(), String> {
            let Room { handle, transport } = room;
            if let Err(e) = self.command(Command::SetLink(Some(Box::new(transport)))) {
                handle.leave();
                return Err(e);
            }
            lock(&self.lan).room = Some(InRoom { handle, ready: None });
            Ok(())
        }

        /// Creates a room on this PC.
        ///
        /// # Errors
        /// If no game runs or the ports cannot be bound.
        pub fn host_room(&self) -> Result<(), String> {
            let room = Room::host(&self.room_config(&self.lan_config())?)
                .map_err(|e| format!("Cannot host: {e}"))?;
            self.enter(room)
        }

        /// Starts joining the room at `ip` (connecting runs on its own
        /// thread; the result shows up in [`Lan::error`] / [`Lan::room`]).
        pub fn join_room(self: &Arc<Self>, ip: &str) -> Result<(), String> {
            let ip: IpAddr = ip.trim().parse().map_err(|_| "Not an IP address".to_owned())?;
            let mut config = self.lan_config();
            config.last_host_ip = ip.to_string();
            self.set_lan_config(&config);
            let cfg = self.room_config(&config)?;
            lock(&self.lan).joining = true;
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(Room::join(&cfg, ip));
            });
            let front = Arc::clone(self);
            std::thread::spawn(move || {
                let result = rx
                    .recv()
                    .map_err(|_| "The join thread stopped".to_owned())
                    .and_then(|r| r.map_err(|e| format!("Cannot join {ip}: {e}")))
                    .and_then(|room| front.enter(room));
                let mut lan = lock(&front.lan);
                lan.joining = false;
                lan.error = result.err();
            });
            Ok(())
        }

        /// Leaves the room and removes the link from a running core.
        pub fn leave_room(&self) {
            let Some(room) = lock(&self.lan).room.take() else {
                return;
            };
            room.handle.leave();
            if self.shared.is_running() {
                let _ = self.command(Command::SetLink(None));
            }
        }

        /// Loading a state in a room would desync the others.
        ///
        /// # Errors
        /// While in a room.
        pub fn refuse_in_room(&self) -> Result<(), String> {
            if lock(&self.lan).room.is_some() {
                return Err("Leave the LAN room before loading a state".to_owned());
            }
            Ok(())
        }

        /// Keeps the room's MP-ready flag current and notices a closed room.
        pub fn room_tick(&self) {
            let mut lan = lock(&self.lan);
            let Some(room) = &mut lan.room else {
                return;
            };
            if room.handle.has_left() {
                lan.error =
                    room.handle.last_error().or_else(|| Some("The room was closed".to_owned()));
                lan.room = None;
                drop(lan);
                if self.shared.is_running() {
                    let _ = self.command(Command::SetLink(None));
                }
                return;
            }
            let ours = self.shared.fingerprint();
            let host = room.handle.players().into_iter().find(|p| p.is_host);
            let ready = ours.is_some() && host.map(|p| p.rom_fingerprint) == ours;
            if room.ready != Some(ready) {
                room.handle.set_mp_ready(ready);
                room.ready = Some(ready);
            }
        }

        /// The room for the dialog, the join state and the last error.
        pub fn room_view(&self) -> (Option<RoomView>, bool, Option<String>) {
            let lan = lock(&self.lan);
            let view = lan.room.as_ref().map(|r| {
                let h = &r.handle;
                let ours = self.shared.fingerprint();
                let hints = h.link_hints();
                let params = h.link_params();
                RoomView {
                    name: h.room_name(),
                    host: h.is_host(),
                    self_id: h.self_id(),
                    players: h
                        .players()
                        .iter()
                        .map(|p: &PlayerView| {
                            let same = Some(p.rom_fingerprint) == ours;
                            let rtt =
                                if p.rtt_ms > 0 { format!("{} ms", p.rtt_ms) } else { "-".into() };
                            (
                                p.id,
                                p.name.clone(),
                                p.is_host,
                                p.mp_ready && same,
                                rtt,
                                f32::from(p.fps_x10) / 10.,
                            )
                        })
                        .collect(),
                    link: (params.auto, hints.runahead_us, hints.recv_timeout.as_millis() as u16),
                    dropped: h.dropped_stale(),
                }
            });
            (view, lan.joining, lan.error.clone())
        }

        /// Host only: automatic link speed, or fixed run-ahead / timeout.
        pub fn set_link_speed(&self, auto: bool, runahead_us: u32, recv_timeout_ms: u16) {
            if let Some(room) = &lock(&self.lan).room {
                room.handle.set_link_params(LinkParams {
                    auto,
                    runahead_us,
                    recv_timeout_ms,
                    ..room.handle.link_params()
                });
            }
        }

        pub fn clear_lan_error(&self) {
            lock(&self.lan).error = None;
        }
    }
}

#[cfg(feature = "cogi")]
mod stub {
    use super::*;

    /// No LAN play on this core.
    #[derive(Default)]
    pub struct Lan;

    #[allow(
        clippy::unused_self,
        clippy::missing_const_for_fn,
        reason = "the same methods as with LAN play, doing nothing"
    )]
    impl Frontend {
        pub fn leave_room(&self) {}

        pub fn refuse_in_room(&self) -> Result<(), String> {
            Ok(())
        }

        pub fn room_tick(&self) {}
    }
}
#[cfg(feature = "cogi")]
pub use stub::Lan;

/// A lock on the frontend's LAN state (keeps the type name in one place).
pub type LanState = Mutex<Lan>;

/// The Multiplayer dialog.
#[derive(PartialEq)]
pub struct LanView;

#[cfg(feature = "cogi")]
impl Component for LanView {
    fn render(&self) -> impl IntoElement {
        rect().width(Size::fill()).spacing(8.).child(REASON.unwrap_or_default()).child(note(
            "System > Multiplayer > Launch new instance starts another emulator window.",
        ))
    }
}

#[cfg(feature = "lunaris")]
impl Component for LanView {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let front = std::sync::Arc::clone(&ctx.front);
        let config = front.lan_config();
        let name = use_state(|| config.player_name.clone());
        let room_name = use_state(|| config.room_name.clone());
        let host_ip = use_state(|| config.last_host_ip.clone());
        // Redraw twice a second for the player list.
        let mut tick = use_state(|| 0_u32);
        use_hook(move || {
            spawn(async move {
                loop {
                    async_io::Timer::after(std::time::Duration::from_millis(500)).await;
                    *tick.write() += 1;
                }
            });
        });
        let _ = tick.read();
        let (room, joining, error) = front.room_view();
        let running = ctx.status.read().running;

        let save = {
            let front = std::sync::Arc::clone(&front);
            move || {
                let mut c = front.lan_config();
                c.player_name = name.peek().trim().to_owned();
                c.room_name = room_name.peek().trim().to_owned();
                front.set_lan_config(&c);
            }
        };

        let mut out = rect().width(Size::fill());
        if let Some(e) = &error {
            let f = std::sync::Arc::clone(&front);
            out = out.child(
                rect()
                    .horizontal()
                    .spacing(8.)
                    .padding((0., 0., 6., 0.))
                    .color(theme::rgb(DANGER))
                    .font_size(12.5)
                    .child(e.clone())
                    .child(
                        rect()
                            .color(theme::rgb(TEXT_DIM))
                            .on_press(move |_| f.clear_lan_error())
                            .child("✕"),
                    ),
            );
        }

        match room {
            None => {
                let host_front = std::sync::Arc::clone(&front);
                let host_ctx = ctx.clone();
                let host_save = save.clone();
                let join_front = std::sync::Arc::clone(&front);
                let join_ctx = ctx;
                out = out
                    .child(section("you"))
                    .child(field("Player name", Input::new(name).width(Size::px(220.))))
                    .child(section("host a room"))
                    .child(field("Room name", Input::new(room_name).width(Size::px(220.))))
                    .child(field(
                        "",
                        Button::new()
                            .filled()
                            .enabled(running)
                            .on_press(move |_| {
                                host_save();
                                if let Err(e) = host_front.host_room() {
                                    host_ctx.show_error(e);
                                }
                                host_ctx.refresh();
                            })
                            .child("Create room"),
                    ))
                    .child(section("join a room"))
                    .child(field(
                        "Host IP",
                        rect()
                            .horizontal()
                            .spacing(8.)
                            .cross_align(Alignment::Center)
                            .child(Input::new(host_ip).width(Size::px(160.)))
                            .child(
                                Button::new()
                                    .enabled(running && !joining)
                                    .on_press(move |_| {
                                        save();
                                        if let Err(e) = join_front.join_room(&host_ip.peek()) {
                                            join_ctx.show_error(e);
                                        }
                                    })
                                    .child(if joining { "Joining…" } else { "Join" }),
                            ),
                    ))
                    .child(note(if running {
                        "Everyone runs the same game. On one PC: System > Multiplayer > Launch new instance, Create room in one window and Join 127.0.0.1 in the other."
                    } else {
                        "Start a game first: the room compares everyone's ROM."
                    }));
            }
            Some(room) => {
                out = out
                    .child(field(
                        "Room",
                        rect().child(format!(
                            "{} · {} player(s) · you are {}",
                            room.name,
                            room.players.len(),
                            if room.host { "the host" } else { "a guest" }
                        )),
                    ))
                    .child(section("players"))
                    .children(room.players.iter().map(|(id, pname, host, ready, rtt, fps)| {
                        rect()
                            .key(*id)
                            .horizontal()
                            .spacing(10.)
                            .padding((3., 0.))
                            .font_size(12.5)
                            .child(rect().width(Size::px(24.)).child(id.to_string()))
                            .child(rect().width(Size::px(170.)).child(format!(
                                "{pname}{}{}",
                                if *host { " ★" } else { "" },
                                if *id == room.self_id { " (you)" } else { "" }
                            )))
                            .child(
                                rect()
                                    .width(Size::px(90.))
                                    .color(theme::rgb(if *ready { SUCCESS } else { WARNING }))
                                    .child(if *ready { "ready" } else { "other ROM" }),
                            )
                            .child(rect().width(Size::px(70.)).child(format!("{fps:.1} fps")))
                            .child(rect().color(theme::rgb(TEXT_DIM)).child(rtt.clone()))
                            .into_element()
                    }))
                    .child(section("link"));
                let (auto, runahead, timeout) = room.link;
                if room.host {
                    let f = std::sync::Arc::clone(&front);
                    out = out.child(field(
                        "Auto link speed",
                        Switch::new(WritableSetter::new(auto, move |on: bool| {
                            f.set_link_speed(on, runahead, timeout);
                        })),
                    ));
                }
                out = out.child(note(&format!(
                    "Run-ahead {runahead} µs · receive timeout {timeout} ms{}",
                    if room.dropped > 0 {
                        format!(" · {} stale frames dropped", room.dropped)
                    } else {
                        String::new()
                    }
                )));
                let leave_front = std::sync::Arc::clone(&front);
                let leave_ctx = ctx;
                out = out.child(WifiStatus).child(
                    rect().margin((10., 0., 0., 0.)).child(
                        Button::new()
                            .on_press(move |_| {
                                leave_front.leave_room();
                                leave_ctx.refresh();
                            })
                            .child("Leave room"),
                    ),
                );
            }
        }
        out
    }
}

/// How far the emulated Wi-Fi got (the room can be fine while the game
/// never finds the other DS).
#[cfg(feature = "lunaris")]
#[derive(PartialEq)]
struct WifiStatus;

#[cfg(feature = "lunaris")]
impl Component for WifiStatus {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let mut lines = use_state(String::new);
        let front = std::sync::Arc::clone(&ctx.front);
        use_hook(move || {
            spawn(async move {
                loop {
                    // Briefly: a stalled link must not freeze the window.
                    let status = front
                        .command_within(
                            crate::frontend::Command::WifiStatus,
                            std::time::Duration::from_millis(100),
                        )
                        .unwrap_or_default();
                    lines.set(status);
                    async_io::Timer::after(std::time::Duration::from_millis(500)).await;
                }
            });
        });
        let text = lines.read().clone();
        rect().width(Size::fill()).child(section("wi-fi link")).children(text.lines().map(|l| {
            let ok = l.starts_with("OK");
            rect()
                .font_size(11.5)
                .color(theme::rgb(if ok { SUCCESS } else { TEXT_DIM }))
                .child(l.to_owned())
                .into_element()
        }))
    }
}
