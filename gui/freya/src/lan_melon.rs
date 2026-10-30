//! LAN play on the melonDS core: melon_egui's link (`melon_lan`), two
//! consoles over UDP.
//!
//! ```text
//!  Host: LanHost::accept(bind)       Join: LanGuest::connect(address)
//!     (own thread: waits for the other side; the game keeps running)
//!       handshake done -> the same ROM restarts on the link
//!                         (Frontend::start_linked -> SaveHost::network)
//!       the emulator thread paces to LinkPace (Core::link_rate)
//!  Leave -> the ROM restarts without the link
//! ```
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

use melon_lan::{LanGuest, LanHost, LinkPace, LinkStats, Tuning};
use serde::{Deserialize, Serialize};

use super::*;
use crate::dialogs::{fold, hint, stepper};

/// The port when an address has none (melon_egui's).
const LAN_PORT: u16 = 7064;

/// A link the melonDS core is booted on (passed through `Boot::link`).
pub struct MelonLink {
    pub host: Box<dyn melonds::Host>,
    /// The frame rate the link sustains (`None`: in-process airwaves).
    pub pace: Option<LinkPace>,
}

/// The live link, for the dialog.
pub struct Linked {
    pub hosting: bool,
    pub local: String,
    pub remote: String,
    stats: Box<dyn Fn() -> LinkStats + Send>,
}

/// LAN state of the frontend.
#[derive(Default)]
pub struct Lan {
    pub link: Option<Linked>,
    /// What a pending host / join is waiting for.
    pub waiting: Option<String>,
    pub error: Option<String>,
    /// Remote Desktop (`crate::remote`).
    pub remote: crate::remote::Remote,
}

/// The addresses typed last (`freya_lan.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MelonLanConfig {
    /// Where a host listens.
    pub bind: String,
    /// The host a guest joins.
    pub address: String,
    /// The link's knobs for the next connection (melon_egui's VPN tuning).
    pub tuning: Tuning,
}

impl Default for MelonLanConfig {
    fn default() -> Self {
        Self {
            bind: format!("0.0.0.0:{LAN_PORT}"),
            address: format!("127.0.0.1:{LAN_PORT}"),
            tuning: Tuning::default(),
        }
    }
}

/// `ip:port`, or an IP alone on [`LAN_PORT`].
fn parse_address(text: &str) -> Result<SocketAddr, String> {
    let text = text.trim();
    text.parse::<SocketAddr>()
        .or_else(|_| text.parse::<IpAddr>().map(|ip| SocketAddr::new(ip, LAN_PORT)))
        .map_err(|_| format!("Not an address: {text:?} (e.g. 192.168.0.10:{LAN_PORT})"))
}

/// What the dialog shows.
pub struct RoomView {
    pub hosting: bool,
    pub local: String,
    pub remote: String,
    pub stats: LinkStats,
}

impl Frontend {
    fn melon_lan_path(&self) -> PathBuf {
        self.instance.root().join("freya_lan.json")
    }

    pub fn melon_lan_config(&self) -> MelonLanConfig {
        let mut config: MelonLanConfig = std::fs::read_to_string(self.melon_lan_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        config.tuning.normalize();
        config
    }

    pub fn set_melon_lan_config(&self, config: &MelonLanConfig) {
        if let Ok(text) = serde_json::to_string_pretty(config) {
            let _ = std::fs::write(self.melon_lan_path(), text);
        }
    }

    /// Hosts (`true`, listening on `address`) or joins (`false`, the host
    /// at `address`): the handshake runs on its own thread, then the
    /// running ROM restarts on the link.
    ///
    /// # Errors
    /// If no game runs, a link is up or pending, or `address` is invalid.
    pub fn open_link(self: &Arc<Self>, hosting: bool, address: &str) -> Result<(), String> {
        let rom = lock(&self.rom).clone().ok_or("Start a game first")?;
        let addr = parse_address(address)?;
        {
            let lan = lock(&self.lan);
            if lan.link.is_some() || lan.waiting.is_some() {
                return Err("Already linked or connecting".to_owned());
            }
        }
        let mut config = self.melon_lan_config();
        if hosting {
            config.bind = address.trim().to_owned();
        } else {
            config.address = address.trim().to_owned();
        }
        self.set_melon_lan_config(&config);
        let tuning = config.tuning;
        {
            let mut lan = lock(&self.lan);
            lan.error = None;
            lan.waiting = Some(if hosting {
                format!("Waiting for a guest on {addr}…")
            } else {
                format!("Connecting to {addr}…")
            });
        }

        let front = Arc::clone(self);
        let spawned = std::thread::Builder::new().name("melon-lan".to_owned()).spawn(move || {
            let linked = if hosting {
                LanHost::accept(addr, tuning).and_then(|link| {
                    Ok((
                        link.local_addr()?.to_string(),
                        link.remote_addr().to_string(),
                        Box::new(link.stats_reader()) as Box<dyn Fn() -> LinkStats + Send>,
                        link.pace(),
                        Box::new(link) as Box<dyn melonds::Host>,
                    ))
                })
            } else {
                let local = SocketAddr::from(([0, 0, 0, 0], 0));
                LanGuest::connect(local, addr, tuning).and_then(|link| {
                    Ok((
                        link.local_addr()?.to_string(),
                        addr.to_string(),
                        Box::new(link.stats_reader()) as Box<dyn Fn() -> LinkStats + Send>,
                        link.pace(),
                        Box::new(link) as Box<dyn melonds::Host>,
                    ))
                })
            };
            let result =
                linked.map_err(|e| e.to_string()).and_then(|(local, remote, stats, pace, host)| {
                    front
                        .start_linked(&rom, Some(Box::new(MelonLink { host, pace: Some(pace) })))?;
                    Ok(Linked { hosting, local, remote, stats })
                });
            let mut lan = lock(&front.lan);
            lan.waiting = None;
            match result {
                Ok(linked) => lan.link = Some(linked),
                Err(e) => lan.error = Some(format!("LAN: {e}")),
            }
        });
        if let Err(e) = spawned {
            lock(&self.lan).waiting = None;
            return Err(e.to_string());
        }
        Ok(())
    }

    /// Whether a LAN link is up (Remote Desktop needs the airwaves instead).
    pub fn lan_session_active(&self) -> bool {
        lock(&self.lan).link.is_some()
    }

    /// Forgets the link (it goes with the core: stop, reset, another game).
    pub fn leave_room(&self) {
        lock(&self.lan).link = None;
        self.drop_remote_session();
    }

    /// Leave: the running ROM restarts without the link.
    ///
    /// # Errors
    /// If the ROM cannot be restarted.
    pub fn unlink(&self) -> Result<(), String> {
        let rom = lock(&self.rom).clone();
        self.leave_room();
        rom.map_or(Ok(()), |rom| self.start(&rom))
    }

    /// Loading a state on a link would desync the other console.
    ///
    /// # Errors
    /// While linked.
    pub fn refuse_in_room(&self) -> Result<(), String> {
        if lock(&self.lan).link.is_some() {
            return Err("Leave the LAN link before loading a state".to_owned());
        }
        Ok(())
    }

    #[expect(clippy::unused_self, reason = "same call as with the Lunaris core")]
    pub const fn room_tick(&self) {}

    /// The link for the dialog, what is pending and the last error.
    pub fn room_view(&self) -> (Option<RoomView>, Option<String>, Option<String>) {
        let lan = lock(&self.lan);
        let view = lan.link.as_ref().map(|l| RoomView {
            hosting: l.hosting,
            local: l.local.clone(),
            remote: l.remote.clone(),
            stats: (l.stats)(),
        });
        (view, lan.waiting.clone(), lan.error.clone())
    }

    pub fn clear_lan_error(&self) {
        lock(&self.lan).error = None;
    }
}

/// The Multiplayer dialog: LAN play, then Remote Desktop.
impl Component for LanView {
    fn render(&self) -> impl IntoElement {
        rect()
            .width(Size::fill())
            .child(LanPart)
            .child(section("remote desktop"))
            .child(crate::remote::RemotePanel)
    }
}

/// LAN play (two PCs, one console each).
#[derive(PartialEq)]
struct LanPart;

impl Component for LanPart {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let front = Arc::clone(&ctx.front);
        let config = front.melon_lan_config();
        let bind = use_state(|| config.bind.clone());
        let address = use_state(|| config.address.clone());
        // Redraw twice a second for the link stats.
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
        let (link, waiting, error) = front.room_view();
        let running = ctx.status.read().running;

        let mut out = rect().width(Size::fill());
        if let Some(e) = error {
            let f = Arc::clone(&front);
            out = out.child(
                rect()
                    .horizontal()
                    .spacing(8.)
                    .padding((0., 0., 6., 0.))
                    .color(theme::rgb(DANGER))
                    .font_size(12.5)
                    .child(e)
                    .child(
                        rect()
                            .color(theme::rgb(TEXT_DIM))
                            .on_press(move |_| f.clear_lan_error())
                            .child("✕"),
                    ),
            );
        }

        if let Some(link) = link {
            let s = link.stats;
            let leave_front = Arc::clone(&front);
            let leave_ctx = ctx;
            return out
                .child(field(
                    "Link",
                    rect().child(format!(
                        "{} · {} ↔ {}",
                        if link.hosting { "hosting" } else { "joined" },
                        link.local,
                        link.remote
                    )),
                ))
                .child(section("link"))
                .child(field(
                    "Round trip",
                    rect().child(format!(
                        "{:.1} ms (jitter {:.1} ms, reply budget {:.1} ms)",
                        s.rtt_ms, s.jitter_ms, s.budget_ms
                    )),
                ))
                .child(field(
                    "Pace",
                    rect()
                        .color(theme::rgb(if s.sustainable_fps >= 59.0 {
                            SUCCESS
                        } else {
                            WARNING
                        }))
                        .child(format!("{:.1} fps sustainable", s.sustainable_fps)),
                ))
                .child(field(
                    "Rounds",
                    rect().child(format!(
                        "{} answered · {} timed out · {} stale replies",
                        s.rounds_answered, s.rounds_timed_out, s.stale_replies
                    )),
                ))
                .child(field(
                    "Datagrams",
                    rect().child(format!(
                        "{} sent · {} received · {} duplicates dropped",
                        s.datagrams_sent, s.datagrams_received, s.duplicates_dropped
                    )),
                ))
                .child(field(
                    "Wireless",
                    rect().child(if s.wireless_on { "on (the game uses it)" } else { "off" }),
                ))
                .child(
                    rect().margin((10., 0., 0., 0.)).child(
                        Button::new()
                            .on_press(move |_| {
                                if let Err(e) = leave_front.unlink() {
                                    leave_ctx.show_error(e);
                                }
                                leave_ctx.refresh();
                            })
                            .child("Leave (restart without the link)"),
                    ),
                );
        }

        if let Some(w) = waiting {
            return out.child(rect().padding((8., 0.)).color(theme::rgb(WARNING)).child(w)).child(
                note("The game keeps running; it restarts on the link once both sides meet."),
            );
        }

        let host_front = Arc::clone(&front);
        let host_ctx = ctx.clone();
        let join_front = Arc::clone(&front);
        let join_ctx = ctx;
        out.child(section("host"))
            .child(field(
                "Listen on",
                rect()
                    .horizontal()
                    .spacing(8.)
                    .cross_align(Alignment::Center)
                    .child(Input::new(bind).width(Size::px(180.)))
                    .child(
                        Button::new()
                            .filled()
                            .enabled(running)
                            .on_press(move |_| {
                                if let Err(e) = host_front.open_link(true, &bind.peek()) {
                                    host_ctx.show_error(e);
                                }
                            })
                            .child("Host"),
                    ),
            ))
            .child(section("join"))
            .child(field(
                "Host address",
                rect()
                    .horizontal()
                    .spacing(8.)
                    .cross_align(Alignment::Center)
                    .child(Input::new(address).width(Size::px(180.)))
                    .child(
                        Button::new()
                            .enabled(running)
                            .on_press(move |_| {
                                if let Err(e) = join_front.open_link(false, &address.peek()) {
                                    join_ctx.show_error(e);
                                }
                            })
                            .child("Join"),
                    ),
            ))
            .child(note(if running {
                "Both consoles run the same game. On one PC: System > Multiplayer > Launch new instance, Host in one window and Join 127.0.0.1 in the other."
            } else {
                "Start a game first: the link restarts it on both consoles."
            }))
            .child(LinkTuning)
    }
}

/// The link's knobs for the next connection: what a VPN needs varies a lot,
/// so they are editable, folded by default (melon_egui's VPN tuning).
#[derive(PartialEq)]
struct LinkTuning;

impl Component for LinkTuning {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let open = use_state(|| false);
        let tuning = use_state(|| ctx.front.melon_lan_config().tuning);
        // Changes, clamps, saves and shows the knobs.
        let set = {
            let front = Arc::clone(&ctx.front);
            Rc::new(move |change: &dyn Fn(&mut Tuning)| {
                // A copy of the handle: `Fn`, so the `Rc` can call it.
                let mut tuning = tuning;
                let mut t = *tuning.peek();
                change(&mut t);
                t.normalize();
                let mut config = front.melon_lan_config();
                config.tuning = t;
                front.set_melon_lan_config(&config);
                tuning.set(t);
            })
        };
        let on = |change: fn(&mut Tuning)| {
            let set = Rc::clone(&set);
            move |_: Event<PressEventData>| set(&change)
        };

        let out = rect().width(Size::fill()).child(fold(open, "VPN tuning"));
        if !*open.read() {
            return out;
        }
        let t = *tuning.read();
        let pace_set = Rc::clone(&set);
        out.child(note("Applies to the next connection."))
            .child(field(
                "Min reply budget",
                stepper(
                    format!("{} ms", t.min_budget_ms),
                    on(|t| t.min_budget_ms = t.min_budget_ms.saturating_sub(1)),
                    on(|t| t.min_budget_ms = t.min_budget_ms.saturating_add(1)),
                    "the reply wait never goes below this",
                ),
            ))
            .child(field(
                "Max reply budget",
                stepper(
                    format!("{} ms", t.max_budget_ms),
                    on(|t| t.max_budget_ms = t.max_budget_ms.saturating_sub(10)),
                    on(|t| t.max_budget_ms = t.max_budget_ms.saturating_add(10)),
                    "the worst link it still tries to play over",
                ),
            ))
            .child(field(
                "Jitter factor",
                stepper(
                    format!("× {}", t.jitter_factor),
                    on(|t| t.jitter_factor = t.jitter_factor.saturating_sub(1)),
                    on(|t| t.jitter_factor = t.jitter_factor.saturating_add(1)),
                    "multiples of the jitter added to the wait",
                ),
            ))
            .child(field(
                "Reply copies",
                stepper(
                    t.reply_copies.to_string(),
                    on(|t| t.reply_copies = t.reply_copies.saturating_sub(1)),
                    on(|t| t.reply_copies = t.reply_copies.saturating_add(1)),
                    "each reply and CMD goes out this many times",
                ),
            ))
            .child(field(
                "Batch window",
                stepper(
                    format!("{} ms", t.batch_window_ms),
                    on(|t| t.batch_window_ms = t.batch_window_ms.saturating_sub(1)),
                    on(|t| t.batch_window_ms = t.batch_window_ms.saturating_add(1)),
                    "other frames wait this long to share a datagram (0: off)",
                ),
            ))
            .child(field(
                "Pace to the link",
                rect()
                    .horizontal()
                    .spacing(10.)
                    .cross_align(Alignment::Center)
                    .child(Switch::new(WritableSetter::new(t.pace_to_link, move |on: bool| {
                        pace_set(&move |t: &mut Tuning| t.pace_to_link = on);
                    })))
                    .child(hint("off: 59.83 fps, rounds the link misses are dropped")),
            ))
            .child(
                rect().margin((6., 0., 0., 0.)).child(
                    Button::new()
                        .on_press(move |_| set(&|t: &mut Tuning| *t = Tuning::default()))
                        .child("Reset to defaults"),
                ),
            )
    }
}
