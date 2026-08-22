//! Remote Desktop mode: both consoles here, picture and sound out, controls
//! back. See [`crate::remote`] for why this beats carrying the wireless.

use super::*;

impl MelonEgui {
    /// Whether a Remote Desktop session is up or being established.
    #[must_use]
    pub const fn remote_running(&self) -> bool {
        self.remote_host.is_some() || self.remote_client.is_some() || self.remote_pending.is_some()
    }

    /// Begin a Remote Desktop session, without blocking the UI thread.
    ///
    /// As host: both consoles will run here, and the second one's picture and
    /// sound go out to whoever connects. As client: this window stops being an
    /// emulator and becomes a screen.
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
        let bind = self.lan_bind_address.clone();
        let address = self.lan_guest_address.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name(
                if host { "melon-egui-remote-host" } else { "melon-egui-remote-client" }.to_owned(),
            )
            .spawn(move || {
                let result = if host {
                    parse_remote_address(&bind, tuning.port).and_then(|addr| {
                        crate::remote::RemoteHost::accept(addr, tuning)
                            .map(|host| RemoteSession::Host(Box::new(host)))
                            .map_err(|error| format!("Remote Desktop host failed: {error}"))
                    })
                } else {
                    parse_remote_address(&address, tuning.port).and_then(|remote| {
                        // Any local port: the client only ever talks to the one
                        // host, which answers wherever the hello came from.
                        let local = std::net::SocketAddr::from(([0, 0, 0, 0], 0));
                        crate::remote::RemoteClient::connect(local, remote, tuning)
                            .map(|client| RemoteSession::Client(Box::new(client)))
                            .map_err(|error| format!("Remote Desktop client failed: {error}"))
                    })
                };
                let _ = sender.send(result);
            })
            .map_err(|error| format!("cannot start a Remote Desktop session: {error}"));
        if let Err(error) = spawned {
            self.post_error(error);
            return;
        }
        self.remote_pending = Some(receiver);
        // Saved on the attempt: an address that did not answer is still the one
        // the user meant to type.
        self.persist();
        self.lan_room =
            if host { "Remote Desktop: hosting" } else { "Remote Desktop: joining" }.to_owned();
        // The port shown is the one that will actually be used — see
        // `parse_remote_address`.
        let (address, what) = if host {
            (&self.lan_bind_address, "waiting for a client on")
        } else {
            (&self.lan_guest_address, "connecting to")
        };
        let address = parse_remote_address(address, self.remote_tuning.port)
            .map_or_else(|error| error, |addr| addr.to_string());
        self.lan_status = Notice::quiet(Severity::Info, format!("{what} {address}"));
        self.post(format!("{what} {address}"));
    }

    /// Finish a Remote Desktop session that the connection thread established.
    pub(crate) fn poll_remote(&mut self) {
        // Sampled every repaint so the pane and the menu agree, and so a
        // session that has gone quiet is visible rather than merely stale.
        self.remote_stats = match (&self.remote_host, &self.remote_client) {
            (Some(host), _) => Some(host.stats()),
            (_, Some(client)) => Some(client.stats()),
            _ => None,
        };

        let Some(receiver) = &self.remote_pending else { return };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.remote_pending = None;
                self.post_error("the Remote Desktop worker stopped unexpectedly");
                return;
            }
        };
        self.remote_pending = None;
        match result {
            Ok(RemoteSession::Host(host)) => {
                let host = *host;
                let local = host.local_addr().map_or_else(|_| "?".to_owned(), |a| a.to_string());
                let remote = host.remote_addr();
                self.remote_host = Some(std::sync::Arc::new(host));
                self.mode = Mode::RemoteHost;
                // The remote player's console. Launched *after* the session
                // exists, because the stream is fixed when the thread starts.
                self.close_guest();
                self.launch_instance();
                self.lan_room = "Remote Desktop: hosting".to_owned();
                self.lan_status = Notice::quiet(
                    Severity::Success,
                    format!("Client {remote} connected; listening on {local}"),
                );
                self.post_ok(format!("Remote Desktop: {remote} is playing instance 2"));
            }
            Ok(RemoteSession::Client(client)) => {
                let client = *client;
                // A client emulates nothing, so whatever was running here stops
                // — and its save is flushed on the way out.
                self.emu = None;
                self.drop_link();
                self.close_guest();
                self.textures = None;
                let remote = client.remote_addr();
                self.remote_client = Some(client);
                self.mode = Mode::RemoteClient;
                self.paused = false;
                let local = self
                    .remote_client
                    .as_ref()
                    .and_then(|client| client.local_addr().ok())
                    .map_or_else(|| "?".to_owned(), |addr| addr.to_string());
                self.lan_room = "Remote Desktop: connected".to_owned();
                self.lan_status =
                    Notice::quiet(Severity::Success, format!("Watching {remote} from {local}"));
                self.post_ok(format!("Remote Desktop: connected to {remote}"));
            }
            Err(error) => {
                self.lan_room = "Remote Desktop: offline".to_owned();
                self.lan_status = Notice::quiet(Severity::Error, error.clone());
                self.post_error(error);
            }
        }
    }

    /// End a Remote Desktop session and go back to being an ordinary window.
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

    /// Show the picture and play the sound a host is sending.
    ///
    /// Everything a client does in place of emulating: there is no core here,
    /// so the textures are filled from the decoder and the audio ring from the
    /// network rather than from an [`Emu`].
    pub(crate) fn service_remote_client(&mut self, ctx: &egui::Context) {
        let Some(client) = &self.remote_client else { return };

        // **First**, before anything else in the repaint.
        //
        // Decoding a frame and uploading two textures costs several
        // milliseconds, and until this call moved above them every one of those
        // milliseconds sat between the player moving the stylus and the host
        // hearing about it. Nothing below depends on the controls, so there is
        // no reason for them to wait.
        //
        // The touch is mapped against the screen rectangle from the *previous*
        // repaint, which is what `bottom_screen` holds: this runs before the
        // panel is laid out. That is only ever wrong while the window is being
        // resized, and it self-corrects on the next repaint — a far better
        // trade than paying the decode on every sample.
        // A client has no console of its own, so the speed clicks in this
        // sample are dropped: the speed belongs to the host, which is where the
        // emulation is.
        let pad_keys = self.pads.poll(&self.bindings).keys;
        let keys = if self.listening.is_some() {
            0
        } else {
            ctx.input(|i| self.bindings.key_mask(i)) | pad_keys
        };
        client.send_input(keys, self.sample_touch(ctx));

        if let Some([top, bottom]) = client.take_screens() {
            let filter =
                if self.view.filtering { TextureOptions::LINEAR } else { TextureOptions::NEAREST };
            let images = [
                to_image(&top, self.video.upscale, self.video.upscale_factor()),
                to_image(&bottom, self.video.upscale, self.video.upscale_factor()),
            ];
            match &mut self.textures {
                Some(textures) => {
                    for (texture, image) in textures.iter_mut().zip(images) {
                        texture.set(image, filter);
                    }
                }
                None => {
                    let [t, b] = images;
                    self.textures = Some([
                        ctx.load_texture("remote-top", t, filter),
                        ctx.load_texture("remote-bottom", b, filter),
                    ]);
                }
            }
            self.screens_live = [true, true];
            self.frames_run += 1;
            self.fps_frames += 1;
        }

        // The sound arrives decimated; the resampler upsamples it to the
        // device rate on its way into the ring. See `crate::remote::audio`.
        let (samples, rate) = client.take_audio();
        if let (Ok(audio), false) = (&mut self.audio, samples.is_empty()) {
            audio.push_at(&samples, rate);
        }
    }
}
