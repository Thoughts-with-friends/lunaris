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
    pub(crate) fn start_remote(&mut self, host: bool) {
        if self.remote_pending.is_some() {
            self.post_warn("a Remote Desktop session is already being established");
            return;
        }
        if host && !self.is_loaded() {
            self.post_warn("load a cart first — the host runs both consoles");
            return;
        }
        let tuning = self.remote_tuning;
        // The host binds its box's address; the client dials the other box.
        let (address, what) = if host {
            (self.lan_bind_address.clone(), "waiting for a client on")
        } else {
            (self.lan_guest_address.clone(), "connecting to")
        };
        let job_address = address.clone();
        let name = if host { "melon-egui-remote-host" } else { "melon-egui-remote-client" };
        let spawned = worker::spawn(name, move || {
            let addr = parse_remote_address(&job_address, tuning.port)?;
            if host {
                crate::remote::RemoteHost::accept(addr, tuning)
                    .map(|host| RemoteSession::Host(Box::new(host)))
                    .map_err(|error| format!("Remote Desktop host failed: {error}"))
            } else {
                // Any local port: the host answers wherever the hello came from.
                let local = std::net::SocketAddr::from(([0, 0, 0, 0], 0));
                crate::remote::RemoteClient::connect(local, addr, tuning)
                    .map(|client| RemoteSession::Client(Box::new(client)))
                    .map_err(|error| format!("Remote Desktop client failed: {error}"))
            }
        });
        match spawned {
            Ok(receiver) => self.remote_pending = Some(receiver),
            Err(error) => {
                return self.post_error(format!("cannot start a Remote Desktop session: {error}"));
            }
        }
        // Saved on the attempt, so a retry does not mean typing it again.
        self.persist();
        self.lan_room =
            if host { "Remote Desktop: hosting" } else { "Remote Desktop: joining" }.to_owned();
        // The port shown is the one actually used (see `parse_remote_address`).
        let shown = parse_remote_address(&address, tuning.port)
            .map_or_else(|error| error, |addr| addr.to_string());
        self.lan_status = Notice::quiet(Severity::Info, format!("{what} {shown}"));
        self.post(format!("{what} {shown}"));
    }

    /// Sample the live session's numbers, and finish a session the worker
    /// established.
    pub(crate) fn poll_remote(&mut self) {
        // Every repaint, so the pane and the menu read one consistent set.
        self.remote_stats = match (&self.remote_host, &self.remote_client) {
            (Some(host), _) => Some(host.stats()),
            (_, Some(client)) => Some(client.stats()),
            _ => None,
        };

        let Some(result) = worker::take(&mut self.remote_pending) else { return };
        match result {
            None => self.post_error("the Remote Desktop worker stopped unexpectedly"),
            Some(Ok(RemoteSession::Host(host))) => self.become_remote_host(*host),
            Some(Ok(RemoteSession::Client(client))) => self.become_remote_client(*client),
            Some(Err(error)) => {
                self.lan_room = "Remote Desktop: offline".to_owned();
                self.lan_status = Notice::quiet(Severity::Error, error.clone());
                self.post_error(error);
            }
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
        self.lan_room = "Remote Desktop: hosting".to_owned();
        self.lan_status = Notice::quiet(
            Severity::Success,
            format!("Client {remote} connected; listening on {local}"),
        );
        self.post_ok(format!("Remote Desktop: {remote} is playing instance 2"));
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
        self.lan_room = "Remote Desktop: connected".to_owned();
        self.lan_status =
            Notice::quiet(Severity::Success, format!("Watching {remote} from {local}"));
        self.post_ok(format!("Remote Desktop: connected to {remote}"));
    }

    /// End the session and go back to being an ordinary window.
    pub(crate) fn stop_remote(&mut self) {
        if self.remote_host.is_none() && self.remote_client.is_none() {
            self.post_warn("no Remote Desktop session is running");
            return;
        }
        // The host's second console was the remote player's; it goes with them.
        self.close_guest();
        self.remote_host = None;
        self.remote_client = None;
        self.remote_stats = None;
        self.textures = None;
        self.mode = Mode::Local;
        self.lan_room = "Remote Desktop: offline".to_owned();
        self.lan_status = Notice::quiet(Severity::Info, "No Remote Desktop session");
        self.post("Remote Desktop session ended");
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
