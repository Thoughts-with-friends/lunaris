//! Remote Desktop on the melonDS core (melon_egui's, via `melon_remote`):
//! the host PC runs both consoles and streams the second one's picture and
//! sound; the other PC shows them and sends its controls back.
//!
//! ```text
//!  HOST                                         CLIENT
//!  host_remote(bind)                            join_remote(address)
//!   └ thread: RemoteHost::accept  ←── hello ──── thread: RemoteClient::connect
//!  connected:                                   connected:
//!   ├ Airwaves: this console restarts on seat 0  └ the emulator thread runs a
//!   └ "remote-console" thread: console 2 on        RemoteView "core": keys and
//!     seat 1, keys from the stream,                stylus -> send_input,
//!     picture + sound -> the stream                screens + sound <- stream
//!  Stop / Leave / another game ends the session (both sides).
//! ```
//!
//! Why this beats LAN play over a VPN: `melon_remote`'s crate docs.
use std::{
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use ds_common::frontend::lock;
use freya::prelude::*;
use melon_lan::mp::Airwaves;
use melon_remote::{RemoteClient, RemoteHost, RemoteStats, Tuning};

use crate::{
    backend::{Core, melon::Melon, push_rgba},
    dialogs::{field, note},
    frontend::Frontend,
    lan::MelonLink,
    state::Ctx,
    theme::{self, DANGER, SUCCESS, TEXT_DIM, WARNING},
};

/// The DS frame rate.
const FRAME_RATE: f64 = 33_513_982.0 / 560_190.0;

/// The hosting side: the stream and the second console's thread.
struct Hosting {
    stream: Arc<RemoteHost>,
    stop: Arc<AtomicBool>,
    console: Option<JoinHandle<()>>,
    /// Keeps the airwaves both consoles are on.
    _air: Airwaves,
}

impl Drop for Hosting {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(console) = self.console.take() {
            let _ = console.join();
        }
    }
}

/// Remote Desktop state of the frontend.
#[derive(Default)]
pub struct Remote {
    hosting: Option<Hosting>,
    client: Option<Arc<RemoteClient>>,
    /// A host / join still waiting for the other side, and its cancel flag.
    waiting: Option<(String, Arc<AtomicBool>)>,
    pub error: Option<String>,
}

/// What the dialog shows.
pub struct RemoteView {
    pub hosting: bool,
    pub peer: SocketAddr,
    pub stats: RemoteStats,
}

/// `ip:port`, or an IP alone on `port`.
fn parse(text: &str, port: u16) -> Result<SocketAddr, String> {
    let text = text.trim();
    text.parse::<SocketAddr>()
        .or_else(|_| text.parse::<IpAddr>().map(|ip| SocketAddr::new(ip, port)))
        .map_err(|_| format!("Not an address: {text:?} (e.g. 192.168.0.10:{port})"))
}

/// The second console's save, beside the first one's.
fn second_save(save: &Path) -> PathBuf {
    save.with_extension("2.sav")
}

impl Frontend {
    /// Starts hosting on `bind`: waits (its own thread) for a client, then
    /// runs the second console for it.
    ///
    /// # Errors
    /// If no game runs, a session is up or pending, or `bind` is invalid.
    pub fn host_remote(self: &Arc<Self>, bind: &str) -> Result<(), String> {
        let rom = lock(&self.rom).clone().ok_or("Start a game first")?;
        self.begin(true, bind, move |front, addr, tuning, cancel| {
            let stream = RemoteHost::accept(addr, tuning, cancel).map_err(|e| e.to_string())?;
            front.serve(&rom, Arc::new(stream))
        })
    }

    /// Starts joining the host at `address` (its own thread); once it
    /// answers, this window shows the host's second console.
    ///
    /// # Errors
    /// If a session is up or pending, or `address` is invalid.
    pub fn join_remote(self: &Arc<Self>, address: &str) -> Result<(), String> {
        self.begin(false, address, move |front, addr, tuning, cancel| {
            let local = SocketAddr::from(([0, 0, 0, 0], 0));
            let client = Arc::new(
                RemoteClient::connect(local, addr, tuning, cancel).map_err(|e| e.to_string())?,
            );
            front.watch(addr, &client)?;
            lock(&front.lan).remote.client = Some(client);
            Ok(())
        })
    }

    /// Runs `connect` on a thread of its own, keeping the waiting state.
    fn begin(
        self: &Arc<Self>,
        hosting: bool,
        address: &str,
        connect: impl FnOnce(&Arc<Self>, SocketAddr, Tuning, &AtomicBool) -> Result<(), String>
        + Send
        + 'static,
    ) -> Result<(), String> {
        let tuning = Tuning::default();
        let addr = parse(address, tuning.port)?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut lan = lock(&self.lan);
            let r = &lan.remote;
            if r.hosting.is_some() || r.client.is_some() || r.waiting.is_some() {
                return Err("A Remote Desktop session is already up".to_owned());
            }
            let what = if hosting {
                format!("Waiting for a client on {addr}…")
            } else {
                format!("Connecting to {addr}…")
            };
            lan.remote.waiting = Some((what, Arc::clone(&cancel)));
            lan.remote.error = None;
        }
        let front = Arc::clone(self);
        let name = if hosting { "remote-host" } else { "remote-client" };
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let result = connect(&front, addr, tuning, &cancel);
                let mut lan = lock(&front.lan);
                lan.remote.waiting = None;
                if let Err(e) = result
                    && !cancel.load(Ordering::Relaxed)
                {
                    lan.remote.error = Some(format!("Remote Desktop: {e}"));
                }
            })
            .map(drop)
            .map_err(|e| {
                lock(&self.lan).remote.waiting = None;
                e.to_string()
            })
    }

    /// Host side, once a client said hello: this console restarts on seat 0
    /// of new airwaves, the second one starts on seat 1 for the stream.
    fn serve(&self, rom: &Path, stream: Arc<RemoteHost>) -> Result<(), String> {
        let air = Airwaves::new();
        let link = MelonLink { host: Box::new(air.client(0)), pace: None };
        self.start_linked(rom, Some(Box::new(link)))?;

        let save = second_save(&self.instance.save_path(rom));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_stop, thread_stream) = (Arc::clone(&stop), Arc::clone(&stream));
        let (seat, rom) = (air.client(1), rom.to_owned());
        let (booted, boot_result) = std::sync::mpsc::channel();
        // Built on its own thread: a console is not `Send` (its GL context).
        let handle = std::thread::Builder::new()
            .name("remote-console".to_owned())
            .spawn(move || match Melon::boot(&rom, &save, 1, Some(Box::new(seat)), None) {
                Ok(mut console) => {
                    let _ = booted.send(Ok(()));
                    run_second(&mut console, &thread_stream, &thread_stop);
                }
                Err(e) => {
                    let _ = booted.send(Err(e));
                }
            })
            .map_err(|e| e.to_string())?;
        boot_result.recv().map_err(|_| "The remote console stopped".to_owned())??;
        lock(&self.lan).remote.hosting =
            Some(Hosting { stream, stop, console: Some(handle), _air: air });
        Ok(())
    }

    /// Client side: the emulator thread shows the stream.
    fn watch(&self, host: SocketAddr, client: &Arc<RemoteClient>) -> Result<(), String> {
        self.stop();
        let client = Arc::clone(client);
        let label = PathBuf::from(format!("Remote Desktop {host}"));
        self.run_core(
            &label,
            Box::new(move || {
                Ok(Box::new(RemoteScreen {
                    client,
                    screens: vec![0; 256 * 192 * 2],
                    resampler: None,
                }) as Box<dyn Core>)
            }),
        )
    }

    /// Ends a running session (Stop, another game, Leave): the second
    /// console stops, the client's stream closes.
    pub fn drop_remote_session(&self) {
        let (hosting, client) = {
            let mut lan = lock(&self.lan);
            (lan.remote.hosting.take(), lan.remote.client.take())
        };
        drop(hosting);
        drop(client);
    }

    /// Stop button: cancels a pending host / join, or ends the session
    /// (a host keeps playing its own console; a client's screen goes away).
    pub fn end_remote(&self) {
        let waiting = lock(&self.lan).remote.waiting.take();
        if let Some((_, cancel)) = waiting {
            cancel.store(true, Ordering::Relaxed);
        }
        let client = lock(&self.lan).remote.client.is_some();
        if client {
            self.stop();
        } else {
            self.drop_remote_session();
        }
    }

    /// The session for the dialog, what is pending and the last error.
    pub fn remote_view(&self) -> (Option<RemoteView>, Option<String>, Option<String>) {
        let lan = lock(&self.lan);
        let r = &lan.remote;
        let view = r
            .hosting
            .as_ref()
            .map(|h| RemoteView {
                hosting: true,
                peer: h.stream.remote_addr(),
                stats: h.stream.stats(),
            })
            .or_else(|| {
                r.client.as_ref().map(|c| RemoteView {
                    hosting: false,
                    peer: c.remote_addr(),
                    stats: c.stats(),
                })
            });
        let pending = (r.waiting.as_ref().map(|(w, _)| w.clone()), r.error.clone());
        drop(lan);
        (view, pending.0, pending.1)
    }

    pub fn clear_remote_error(&self) {
        lock(&self.lan).remote.error = None;
    }
}

/// The second console's loop (host side): the remote player's controls in,
/// picture and sound out, at the DS frame rate.
fn run_second(console: &mut Melon, stream: &RemoteHost, stop: &AtomicBool) {
    let frame_time = Duration::from_secs_f64(1.0 / FRAME_RATE);
    let mut next = Instant::now();
    let mut audio = Vec::new();
    let mut frames = 0_u32;
    while !stop.load(Ordering::Relaxed) {
        let input = stream.input();
        let touch = input.touch.map(|(x, y)| (i32::from(x), i32::from(y)));
        console.run_frame(input.keys, touch);
        if let Some((top, bottom)) = console.screens() {
            stream.send_frame(top, bottom);
        }
        audio.clear();
        console.take_raw_audio(&mut audio);
        stream.send_audio(&audio);
        frames += 1;
        if frames.is_multiple_of(60)
            && let Err(e) = console.flush_save()
        {
            log::error!("Remote console save: {e}");
        }
        next += frame_time;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > frame_time * 4 {
            next = now;
        }
    }
    if let Err(e) = console.flush_save() {
        log::error!("Remote console save: {e}");
    }
}

/// The client's "core": shows what the host streams and sends the controls.
struct RemoteScreen {
    client: Arc<RemoteClient>,
    /// The latest picture (0xAARRGGBB, top then bottom).
    screens: Vec<u32>,
    /// From the stream's audio rate to the output's (made on first sound).
    resampler: Option<(u32, crate::backend::melon::Resampler)>,
}

impl Core for RemoteScreen {
    fn run_frame(&mut self, keys: u32, touch: Option<(i32, i32)>) {
        let touch = touch.map(|(x, y)| (x.clamp(0, 255) as u16, y.clamp(0, 191) as u16));
        self.client.send_input(keys, touch);
        if let Some([top, bottom]) = self.client.take_screens() {
            self.screens.clear();
            self.screens.extend_from_slice(&top);
            self.screens.extend_from_slice(&bottom);
        }
    }

    fn take_audio(&mut self, out: &mut Vec<i16>) {
        let (samples, rate) = self.client.take_audio();
        if samples.is_empty() || rate == 0 {
            return;
        }
        if self.resampler.as_ref().is_none_or(|(r, _)| *r != rate) {
            let step = f64::from(rate) / ds_spu::OUTPUT_RATE_HZ;
            self.resampler = Some((rate, crate::backend::melon::Resampler::new(step)));
        }
        if let Some((_, resampler)) = &mut self.resampler {
            resampler.process(&samples, out);
        }
    }

    fn frame(&mut self, rgba: &mut Vec<u8>) {
        push_rgba(&self.screens, rgba);
    }

    fn set_cheats(&mut self, _codes: Vec<Vec<u32>>) {}

    fn flush_save(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// The Remote Desktop part of the Multiplayer dialog.
#[derive(PartialEq)]
pub struct RemotePanel;

impl Component for RemotePanel {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let front = Arc::clone(&ctx.front);
        let port = Tuning::default().port;
        let bind = use_state(|| format!("0.0.0.0:{port}"));
        let address = use_state(|| format!("127.0.0.1:{port}"));
        let mut tick = use_state(|| 0_u32);
        use_hook(move || {
            spawn(async move {
                loop {
                    async_io::Timer::after(Duration::from_millis(500)).await;
                    *tick.write() += 1;
                }
            });
        });
        let _ = tick.read();
        let (view, waiting, error) = front.remote_view();
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
                            .on_press(move |_| f.clear_remote_error())
                            .child("✕"),
                    ),
            );
        }
        let stop_front = Arc::clone(&front);
        let stop_ctx = ctx.clone();
        let stop = Button::new()
            .on_press(move |_| {
                stop_front.end_remote();
                stop_ctx.refresh();
            })
            .child("Stop Remote Desktop");

        if let Some(v) = view {
            let s = v.stats;
            let role = if v.hosting {
                "hosting: this PC runs both consoles, the second is streamed"
            } else {
                "showing the host's second console"
            };
            return out
                .child(field("Session", rect().child(format!("{role} · peer {}", v.peer))))
                .child(field(
                    "Link",
                    rect().color(theme::rgb(if s.peer_ready() { SUCCESS } else { WARNING })).child(
                        format!(
                            "{:.1} ms round trip · video {:.1} fps, {:.2} Mbit/s",
                            s.rtt_ms,
                            s.video_fps,
                            s.video_megabits_per_second()
                        ),
                    ),
                ))
                .child(rect().margin((8., 0., 0., 0.)).child(stop));
        }
        if let Some(w) = waiting {
            return out
                .child(rect().padding((8., 0.)).color(theme::rgb(WARNING)).child(w))
                .child(rect().child(stop))
                .child(note("Either side may start first; it waits until the other answers."));
        }

        let host_front = Arc::clone(&front);
        let host_ctx = ctx.clone();
        let join_front = Arc::clone(&front);
        let join_ctx = ctx;
        out.child(field(
            "Host on",
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
                            if let Err(e) = host_front.host_remote(&bind.peek()) {
                                host_ctx.show_error(e);
                            }
                        })
                        .child("Host"),
                ),
        ))
        .child(field(
            "Join host",
            rect()
                .horizontal()
                .spacing(8.)
                .cross_align(Alignment::Center)
                .child(Input::new(address).width(Size::px(180.)))
                .child(
                    Button::new()
                        .on_press(move |_| {
                            if let Err(e) = join_front.join_remote(&address.peek()) {
                                join_ctx.show_error(e);
                            }
                        })
                        .child("Join"),
                ),
        ))
        .child(note(
            "The host runs both consoles (start the game there first) and streams the second \
             one: smoother than LAN play over a slow link or VPN. The client needs no ROM.",
        ))
    }
}
