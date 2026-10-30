//! LAN play (System > Multiplayer > Host / join LAN game): the egui GUI's
//! LAN play on melonDS's LAN backend (`ds_net` / `ds_net_gui`), for the
//! Lunaris core.
//!
//! ```text
//!  Host: LanSession::host        Join: LanSession::join  (worker thread)
//!  every 0.5 s (input loop): room_tick -> join result, ENet pump, and a
//!     fresh ds_net::NetLink --Command::SetLink--> core when a session starts
//!  stop / reset / another game: the link goes with the core, the session
//!     is left
//! ```
//!
//! melonDS's LAN backend is one session per process: play with another PC
//! (or another process on this PC, `Launch new instance`).
use std::sync::Mutex;

use ds_common::frontend::lock;
use freya::prelude::*;

use crate::{
    dialogs::{field, note, section},
    frontend::Frontend,
    state::Ctx,
    theme::{self, DANGER, SUCCESS, TEXT_DIM, WARNING},
};

/// Why this build has no LAN play (`None`: it has; every core has now).
pub const REASON: Option<&str> = None;

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
    use std::{path::PathBuf, time::Duration};

    use ds_net_gui::{LanSession, PlayerStatus, SessionState};
    use lunaris_gui_common::config::LanConfig;

    use super::*;
    use crate::frontend::Command;

    /// How long the console waits for its peers per frame before carrying
    /// on without them (`ds_net::NetLink`).
    const STALL_BUDGET: Duration = Duration::from_millis(100);

    /// LAN state of the frontend: melonDS's LAN session (`ds_net`, one per
    /// process), made when first used.
    #[derive(Default)]
    pub struct Lan {
        session: Option<LanSession>,
        /// Session generation the running core's link was made in.
        linked: Option<u64>,
        pub error: Option<String>,
    }

    impl Lan {
        fn session(&mut self) -> &mut LanSession {
            self.session.get_or_insert_with(LanSession::new)
        }

        fn in_session(&self) -> bool {
            self.session.as_ref().is_some_and(|s| s.state().in_session())
        }
    }

    /// What the dialog shows of the session.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RoomView {
        pub host: bool,
        /// The host's address (guest), or this machine's LAN address (host).
        pub address: String,
        /// Name, status, address, this console, ping.
        pub players: Vec<(i32, String, &'static str, String, bool, u32)>,
    }

    fn config_path(front: &Frontend) -> PathBuf {
        front.instance.root().join("freya_lan.json")
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

    impl Frontend {
        /// The LAN settings (`freya_lan.json`).
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

        /// Hosts a session on this PC (UDP 7064).
        ///
        /// # Errors
        /// If no game runs or the port cannot be bound.
        pub fn host_room(&self) -> Result<(), String> {
            if !self.shared.is_running() {
                return Err("Start a game first".to_owned());
            }
            let name = self.lan_config().player_name;
            let _ = self.command(Command::SetLink(None));
            let mut lan = lock(&self.lan);
            lan.linked = None;
            lan.session().host(&name).map_err(|e| format!("Cannot host: {e}"))
        }

        /// Starts joining the session at `address` (connecting runs on a
        /// worker; [`Self::room_tick`] picks the result up).
        ///
        /// # Errors
        /// If no game runs.
        pub fn join_room(&self, address: &str) -> Result<(), String> {
            if !self.shared.is_running() {
                return Err("Start a game first".to_owned());
            }
            let address = address.trim().to_owned();
            let mut config = self.lan_config();
            config.last_host_ip.clone_from(&address);
            self.set_lan_config(&config);
            // The LAN backend is driven by the join worker meanwhile.
            let _ = self.command(Command::SetLink(None));
            let mut lan = lock(&self.lan);
            lan.linked = None;
            lan.error = None;
            lan.session().join(&config.player_name, &address);
            drop(lan);
            Ok(())
        }

        /// Leaves the session and removes the link from a running core.
        pub fn leave_room(&self) {
            let mut lan = lock(&self.lan);
            let Some(session) = &mut lan.session else {
                return;
            };
            session.leave();
            let linked = lan.linked.take().is_some();
            drop(lan);
            if linked && self.shared.is_running() {
                let _ = self.command(Command::SetLink(None));
            }
        }

        /// Loading a state in a session would desync the others.
        ///
        /// # Errors
        /// While in a session.
        pub fn refuse_in_room(&self) -> Result<(), String> {
            if lock(&self.lan).in_session() {
                return Err("Leave the LAN room before loading a state".to_owned());
            }
            Ok(())
        }

        /// Services the session (join result, ENet while no core runs) and
        /// gives the running core a fresh link whenever a session started
        /// (melonDS only puts a console on a medium it powered its Wi-Fi on).
        pub fn room_tick(&self) {
            let running = self.shared.is_running();
            let mut lan = lock(&self.lan);
            let pumped = running && lan.linked.is_some();
            let Some(session) = &mut lan.session else {
                return;
            };
            session.tick(pumped);
            if let SessionState::Failed(e) = session.state() {
                let e = e.clone();
                // Shown once; the session is offline again.
                session.leave();
                lan.error = Some(e);
            }
            let Some(session) = &lan.session else {
                return;
            };
            let generation = session.generation();
            let attach = session.state().in_session()
                && !session.busy()
                && running
                && lan.linked != Some(generation);
            if attach {
                lan.linked = Some(generation);
                drop(lan);
                let link = ds_net::NetLink::new(0).with_stall_budget(Some(STALL_BUDGET));
                if let Err(e) = self.command(Command::SetLink(Some(Box::new(link)))) {
                    lock(&self.lan).error = Some(e);
                }
            }
        }

        /// The session for the dialog, whether a join is connecting, and the
        /// last error.
        pub fn room_view(&self) -> (Option<RoomView>, bool, Option<String>) {
            let lan = lock(&self.lan);
            let Some(session) = &lan.session else {
                return (None, false, lan.error.clone());
            };
            let joining = matches!(session.state(), SessionState::Joining(_));
            let view = match session.state() {
                SessionState::Hosting => Some((
                    true,
                    ds_net_gui::local_ip()
                        .map_or_else(|| "this PC".to_owned(), |ip| ip.to_string()),
                )),
                SessionState::Joined(address) => Some((false, address.clone())),
                _ => None,
            };
            let view = view.map(|(host, address)| RoomView {
                host,
                address,
                players: session
                    .players()
                    .iter()
                    .map(|p| {
                        (
                            p.id,
                            p.name.clone(),
                            status_label(p.status),
                            p.address.to_string(),
                            p.is_local,
                            p.ping_ms,
                        )
                    })
                    .collect(),
            });
            (view, joining, lan.error.clone())
        }

        pub fn clear_lan_error(&self) {
            lock(&self.lan).error = None;
        }
    }
}

/// A lock on the frontend's LAN state (keeps the type name in one place).
pub type LanState = Mutex<Lan>;

/// The Multiplayer dialog.
#[derive(PartialEq)]
pub struct LanView;

#[cfg(feature = "lunaris")]
impl Component for LanView {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let front = std::sync::Arc::clone(&ctx.front);
        let config = front.lan_config();
        let name = use_state(|| config.player_name.clone());
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
                        "Everyone runs the same game (melonDS LAN: UDP 7063-7064). One session per PC: play with another PC on the LAN."
                    } else {
                        "Start a game first."
                    }));
            }
            Some(room) => {
                out = out
                    .child(field(
                        "Session",
                        rect().child(format!(
                            "{} · {} player(s) · you are {}",
                            room.address,
                            room.players.len(),
                            if room.host { "the host" } else { "a guest" }
                        )),
                    ))
                    .child(section("players"))
                    .children(room.players.iter().map(
                        |(id, pname, status, address, local, ping)| {
                            rect()
                                .key(*id)
                                .horizontal()
                                .spacing(10.)
                                .padding((3., 0.))
                                .font_size(12.5)
                                .child(rect().width(Size::px(24.)).child(id.to_string()))
                                .child(rect().width(Size::px(170.)).child(format!(
                                    "{pname}{}",
                                    if *local { " (you)" } else { "" }
                                )))
                                .child(
                                    rect()
                                        .width(Size::px(90.))
                                        .color(theme::rgb(
                                            if *status == "connecting" || *status == "left" {
                                                WARNING
                                            } else {
                                                SUCCESS
                                            },
                                        ))
                                        .child(*status),
                                )
                                .child(rect().width(Size::px(110.)).child(address.clone()))
                                .child(
                                    rect().color(theme::rgb(TEXT_DIM)).child(format!("{ping} ms")),
                                )
                                .into_element()
                        },
                    ));
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
