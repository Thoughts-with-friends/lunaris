//! Remote Desktop mode: both consoles run on the host; the second one's picture
//! and sound are streamed to the client, and the client's controls come back.
//!
//! # Flow
//!
//! ```text
//!  HOST                                        CLIENT
//!  start_remote(true)                          start_remote(false)
//!   └ worker: RemoteHost::accept ←── hello ──── worker: RemoteClient::connect
//!  poll_remote (connected)                     poll_remote (connected)
//!   ├ mode = RemoteHost                         ├ unload the local cart
//!   └ launch the second console with            └ mode = RemoteClient
//!     the session as its "stream"
//!  every frame (guest thread):                 every repaint:
//!   send_frame / send_audio ── video+audio ──→  service_remote_client:
//!   input()  ←───────────────── buttons ─────── 1. send input FIRST
//!                                                2. decode → textures
//!                                                3. audio → speakers
//! ```
//!
//! Why this beats LAN mode over a VPN is explained in [`crate::remote`].

use super::*;

impl MelonEgui {
    /// Whether a Remote Desktop session is up or being established.
    #[must_use]
    pub const fn remote_running(&self) -> bool {
        self.remote_host.is_some() || self.remote_client.is_some() || self.remote_pending.is_some()
    }

    /// Start hosting (`host`) or joining a Remote Desktop session, without
    /// blocking the UI thread.
    ///
    /// Neither end gives up on its own: a host waits for a client and a client
    /// keeps knocking until a host answers, whichever was started first. The
    /// readiness line says which side is still missing, and Stop cancels.
    pub(crate) fn start_remote(&mut self, host: bool) {
        if self.remote_pending.is_some() {
            return self.post_warn(self.i18n().s(K::RdAlreadyPending));
        }
        if host && !self.is_loaded() {
            return self.post_warn(self.i18n().s(K::RdLoadCartFirst));
        }
        let tuning = self.remote_tuning;
        // The host binds its box's address; the client dials the other box.
        let address =
            if host { self.lan_bind_address.clone() } else { self.lan_guest_address.clone() };
        let Ok(addr) = parse_remote_address(&address, tuning.port) else {
            return self.post_error(self.i18n().f(K::InvalidAddress, &[&address]));
        };
        // Translated here and moved in: the worker has no access to the strings.
        let failed = self.i18n().s(if host { K::RdHostFailed } else { K::RdClientFailed });
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&cancel);
        let name = if host { "melon-egui-remote-host" } else { "melon-egui-remote-client" };
        let spawned = worker::spawn(name, move || {
            let session = if host {
                crate::remote::RemoteHost::accept(addr, tuning, &flag)
                    .map(|host| RemoteSession::Host(Box::new(host)))
            } else {
                // Any local port: the host answers wherever the hello came from.
                let local = std::net::SocketAddr::from(([0, 0, 0, 0], 0));
                crate::remote::RemoteClient::connect(local, addr, tuning, &flag)
                    .map(|client| RemoteSession::Client(Box::new(client)))
            };
            session.map_err(|error| crate::i18n::fill(&failed, &[&error]))
        });
        match spawned {
            Ok(receiver) => {
                self.remote_pending = Some(receiver);
                self.remote_cancel = Some(cancel);
                self.remote_pending_host = host;
            }
            Err(error) => return self.post_error(self.i18n().f(K::RdCannotStart, &[&error])),
        }
        // Saved on the attempt, so a retry does not mean typing it again.
        self.persist();
        self.lan_room = self.i18n().s(if host { K::RdRoomHosting } else { K::RdRoomJoining });
        // The port shown is the one actually used (see `parse_remote_address`).
        let what = self.i18n().f(if host { K::RdWaitingOn } else { K::RdConnectingTo }, &[&addr]);
        self.lan_status = Notice::quiet(Severity::Info, what.clone());
        self.post(what);
    }

    /// Sample the live session's numbers, finish a session the worker
    /// established, and refresh the readiness line.
    pub(crate) fn poll_remote(&mut self) {
        // Every repaint, so the pane and the menu read one consistent set.
        self.remote_stats = match (&self.remote_host, &self.remote_client) {
            (Some(host), _) => Some(host.stats()),
            (_, Some(client)) => Some(client.stats()),
            _ => None,
        };
        self.update_probe();
        self.remote_readiness = self.readiness();

        let Some(result) = worker::take(&mut self.remote_pending) else { return };
        self.remote_cancel = None;
        match result {
            None => self.post_error(self.i18n().s(K::RdWorkerStopped)),
            Some(Ok(RemoteSession::Host(host))) => self.become_remote_host(*host),
            Some(Ok(RemoteSession::Client(client))) => self.become_remote_client(*client),
            Some(Err(error)) => {
                self.lan_room = self.i18n().s(K::RdRoomOffline);
                self.lan_status = Notice::quiet(Severity::Error, error.clone());
                self.post_error(error);
            }
        }
    }

    /// Point the prober at the Guest IP box, and let it ask only while this
    /// machine could still *join* someone: not while hosting, and not inside a
    /// session, whose own pings answer the question instead.
    fn update_probe(&self) {
        let Some(probe) = &self.remote_probe else { return };
        let target = parse_remote_address(&self.lan_guest_address, self.remote_tuning.port);
        probe.set_target(target.ok());
        let in_session = self.remote_host.is_some() || self.remote_client.is_some();
        let hosting = self.remote_pending.is_some() && self.remote_pending_host;
        probe.set_active(!in_session && !hosting);
    }

    /// Is the other machine ready? Red when not, green when it is.
    fn readiness(&self) -> Notice {
        use crate::remote::Readiness;
        let tr = self.i18n();
        let ready = |text: String| Notice::quiet(Severity::Success, text);
        let not_ready = |text: String| Notice::quiet(Severity::Error, text);

        // In a session: the session's own pings, which both ends send.
        if let Some(stats) = self.remote_stats {
            return match stats.silent_ms {
                _ if stats.peer_ready() => {
                    ready(tr.f(K::PeerReady, &[&format!("{:.0}", stats.rtt_ms)]))
                }
                Some(ms) => not_ready(tr.f(K::PeerSilent, &[&format!("{:.0}", ms / 1000.0)])),
                None => not_ready(tr.s(K::PeerNeverAnswered)),
            };
        }
        // Hosting and nobody has joined. The host cannot ask anyone: it does
        // not know who will come.
        if self.remote_pending.is_some() && self.remote_pending_host {
            return not_ready(tr.f(K::PeerHostWaiting, &[&self.remote_tuning.port]));
        }
        let Ok(target) = parse_remote_address(&self.lan_guest_address, self.remote_tuning.port)
        else {
            return not_ready(tr.s(K::ProbeBadAddress));
        };
        match self.remote_probe.as_ref().map(crate::remote::Prober::readiness) {
            Some(Readiness::Ready { rtt_ms }) => {
                ready(tr.f(K::ProbeReady, &[&target, &format!("{rtt_ms:.0}")]))
            }
            Some(Readiness::Busy) => not_ready(tr.f(K::ProbeBusy, &[&target])),
            // Joining: the handshake itself is knocking.
            _ if self.remote_pending.is_some() => not_ready(tr.f(K::PeerClientWaiting, &[&target])),
            _ => not_ready(tr.f(K::ProbeSilent, &[&target])),
        }
    }

    /// A client connected: the second console becomes theirs.
    fn become_remote_host(&mut self, host: crate::remote::RemoteHost) {
        let local = host.local_addr().map_or_else(|_| "?".to_owned(), |a| a.to_string());
        let remote = host.remote_addr();
        self.remote_host = Some(std::sync::Arc::new(host));
        self.mode = Mode::RemoteHost;
        // (Re)launched *after* the session exists: the stream a console sends
        // to is fixed when its thread starts.
        self.close_guest();
        self.launch_instance();
        self.lan_room = self.i18n().s(K::RdRoomHosting);
        let status = self.i18n().f(K::RdClientConnected, &[&remote, &local]);
        self.lan_status = Notice::quiet(Severity::Success, status);
        self.post_ok(self.i18n().f(K::RdPlayingInstance2, &[&remote]));
    }

    /// Connected to a host: this window stops emulating and becomes a screen.
    fn become_remote_client(&mut self, client: crate::remote::RemoteClient) {
        // Whatever was running here stops; its save is flushed on the way out.
        self.emu = None;
        self.drop_link();
        self.close_guest();
        self.textures = None;
        let remote = client.remote_addr();
        let local = client.local_addr().map_or_else(|_| "?".to_owned(), |addr| addr.to_string());
        self.remote_client = Some(client);
        self.mode = Mode::RemoteClient;
        self.paused = false;
        self.lan_room = self.i18n().s(K::RdRoomConnected);
        let status = self.i18n().f(K::RdWatching, &[&remote, &local]);
        self.lan_status = Notice::quiet(Severity::Success, status);
        self.post_ok(self.i18n().f(K::RdConnectedTo, &[&remote]));
    }

    /// End the session — or abandon one still being established — and go
    /// back to being an ordinary window.
    pub(crate) fn stop_remote(&mut self) {
        if let Some(cancel) = self.remote_cancel.take() {
            // The worker notices within one read timeout, and its result goes
            // nowhere: the receiver is dropped here.
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.remote_pending = None;
            self.lan_room = self.i18n().s(K::RdRoomOffline);
            self.lan_status = Notice::quiet(Severity::Info, self.i18n().s(K::RdNoSessionStatus));
            return self.post(self.i18n().s(K::RdCancelled));
        }
        if self.remote_host.is_none() && self.remote_client.is_none() {
            return self.post_warn(self.i18n().s(K::RdNoSession));
        }
        // The host's second console was the remote player's; it goes with them.
        self.close_guest();
        self.remote_host = None;
        self.remote_client = None;
        self.remote_stats = None;
        self.textures = None;
        self.mode = Mode::Local;
        self.lan_room = self.i18n().s(K::RdRoomOffline);
        self.lan_status = Notice::quiet(Severity::Info, self.i18n().s(K::RdNoSessionStatus));
        self.post(self.i18n().s(K::RdEnded));
    }

    /// What a client does each repaint instead of emulating.
    pub(crate) fn service_remote_client(&mut self, ctx: &egui::Context) {
        let Some(client) = &self.remote_client else { return };

        // 1. Input goes out **first**, before the costly decode and upload, so
        //    none of that sits between the stylus moving and the host hearing
        //    of it. (Touch uses last repaint's screen rectangle; only wrong
        //    for a repaint while resizing.) Speed clicks are dropped: the
        //    speed belongs to the host.
        let pad_keys = self.pads.poll(&self.bindings).keys;
        client.send_input(self.held_keys(ctx, pad_keys), self.sample_touch(ctx));

        // 2. The newest picture, if any tile changed.
        if let Some([top, bottom]) = client.take_screens() {
            let (video, view) = (self.video, self.view);
            upload_screens(
                ctx,
                &mut self.textures,
                ["remote-top", "remote-bottom"],
                [&top, &bottom],
                &video,
                &view,
            );
            self.screens_live = [true, true];
            self.frames_run += 1;
            self.fps_frames += 1;
        }

        // 3. Sound arrives decimated; `push_at` resamples it for the device.
        let Some(client) = &self.remote_client else { return };
        let (samples, rate) = client.take_audio();
        if let (Ok(audio), false) = (&mut self.audio, samples.is_empty()) {
            audio.push_at(&samples, rate);
        }
    }
}
