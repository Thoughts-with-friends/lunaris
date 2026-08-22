//! LAN mode: emulated wireless carried over the network.
//!
//! Kept beside [`super::remote_session`] rather than merged with it — the two
//! modes answer the same wish in completely different ways, and reading either
//! is easier when the other is not interleaved with it.

use super::*;

impl MelonEgui {
    /// What the live LAN link is doing, for the Wireless pane.
    #[must_use]
    pub fn lan_stats(&self) -> Option<crate::lan::LinkStats> {
        self.lan_stats.as_ref().map(|read| read())
    }

    /// Start a LAN host or guest connection without blocking the UI thread.
    pub(crate) fn start_lan(&mut self, host: bool) {
        if self.lan_pending.is_some() {
            self.post_warn("a LAN connection is already being established");
            return;
        }
        let Some(rom) = self.emu.as_ref().map(|emu| emu.rom_path.clone()) else {
            self.post_warn("load a cart first");
            return;
        };
        self.emu = None;
        self.drop_link();
        self.textures = None;
        self.undo_state = None;
        self.lan_rom = Some(rom);
        let (sender, receiver) = std::sync::mpsc::channel();
        let bind = self.lan_bind_address.clone();
        let address = self.lan_guest_address.clone();
        let bind_for_thread = bind.clone();
        let address_for_thread = address.clone();
        // Read once, here, so that a link keeps whatever tuning it was started
        // with even if the pane is edited while it runs — the two ends have to
        // agree about nothing, but a budget that changes underneath a round in
        // flight is needlessly confusing to reason about.
        let tuning = self.lan_tuning;
        let spawned = std::thread::Builder::new()
            .name(if host { "melon-egui-lan-host" } else { "melon-egui-lan-guest" }.to_owned())
            .spawn(move || {
                let result = if host {
                    parse_lan_address(&bind_for_thread, 7064).and_then(|addr| {
                        crate::lan::LanHost::accept(addr, tuning)
                            .and_then(|transport| {
                                let local_addr = transport.local_addr()?;
                                let remote_addr = transport.remote_addr().to_string();
                                let pace = transport.pace();
                                let transport = std::sync::Arc::new(transport);
                                let reader = std::sync::Arc::clone(&transport);
                                Ok(LanConnection {
                                    local_addr: local_addr.to_string(),
                                    remote_addr,
                                    stats: Box::new(move || reader.stats()),
                                    pace,
                                    host: Box::new(ArcHost(transport)),
                                })
                            })
                            .map_err(|e| format!("LAN host failed: {e}"))
                    })
                } else {
                    let local = "0.0.0.0:0";
                    local.parse().map_err(|e| format!("invalid LAN client address: {e}")).and_then(
                        |local| {
                            parse_lan_address(&address_for_thread, 7064).and_then(|remote| {
                                crate::lan::LanGuest::connect(local, remote, tuning)
                                    .and_then(|transport| {
                                        let local_addr = transport.local_addr()?;
                                        let pace = transport.pace();
                                        let transport = std::sync::Arc::new(transport);
                                        let reader = std::sync::Arc::clone(&transport);
                                        Ok(LanConnection {
                                            local_addr: local_addr.to_string(),
                                            remote_addr: remote.to_string(),
                                            stats: Box::new(move || reader.stats()),
                                            pace,
                                            host: Box::new(ArcHost(transport)),
                                        })
                                    })
                                    .map_err(|e| format!("LAN guest failed: {e}"))
                            })
                        },
                    )
                };
                let _ = sender.send(result);
            })
            .map_err(|e| format!("cannot start LAN connection: {e}"));
        if let Err(error) = spawned {
            self.lan_rom = None;
            self.post_error(error);
            return;
        }
        self.lan_pending = Some(receiver);
        // Saved on the attempt rather than on success: an address that did not
        // answer is still the one the user meant to type, and having to type it
        // again to retry is the annoyance this exists to remove.
        self.persist();
        self.lan_room = if host { "Hosting LAN room" } else { "Joining LAN room" }.to_owned();
        self.lan_status = Notice::quiet(
            Severity::Info,
            if host {
                format!("Checking: waiting for guest on {bind}")
            } else {
                format!("Checking: connecting to {address}")
            },
        );
        self.post(if host {
            format!("waiting for a LAN guest on {bind}")
        } else {
            format!("connecting to LAN host {address}")
        });
    }

    /// Forget the LAN link the last console was on.
    ///
    /// Both handles have to go, and for two different reasons.
    ///
    /// `lan_pace` is the one that bites: [`crate::lan::LinkPace`] is only
    /// updated from inside `mp_recv_replies`, so once the console that made
    /// those calls is gone the last value it wrote **freezes**. A LAN game over
    /// a 100 ms link leaves it at about 10 fps; stopping that game and opening
    /// an ordinary cart would then run the new console at 10 fps for the rest
    /// of the session, with nothing on screen to explain why.
    ///
    /// `lan_stats` holds an `Arc` on the transport, so leaving it behind also
    /// keeps the link's receive and probe threads alive — pinging a peer that
    /// is no longer there — and leaves the Wireless pane reporting a dead
    /// link's counters as though they were live.
    pub(crate) fn drop_link(&mut self) {
        self.lan_stats = None;
        self.lan_pace = None;
    }

    /// Finish a background LAN connection and boot the current cart on it.
    pub(crate) fn poll_lan(&mut self) {
        let Some(receiver) = &self.lan_pending else { return };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.lan_pending = None;
                self.post_error("LAN connection worker stopped unexpectedly");
                return;
            }
        };
        self.lan_pending = None;
        let Some(rom) = self.lan_rom.take() else {
            self.post_warn("LAN connected, but no cart is loaded");
            return;
        };
        match result.and_then(|connection| {
            let local_addr = connection.local_addr.clone();
            let remote_addr = connection.remote_addr.clone();
            let LanConnection { host, stats, pace, .. } = connection;
            Emu::boot_lan(&rom, self.save_dir.as_ref(), self.state_dir.as_ref(), host)
                .map(|emu| (emu, local_addr, remote_addr, stats, pace))
        }) {
            Ok((emu, local_addr, remote_addr, stats, pace)) => {
                self.emu = Some(emu);
                self.lan_stats = Some(stats);
                self.lan_pace = Some(pace);
                self.cheats = mch::load(&Self::cheat_path(&rom)).unwrap_or_default();
                self.select_cheat(None);
                self.applied_cheats = None;
                self.paused = false;
                self.frame_debt = 0.0;
                self.last_tick = Instant::now();
                self.lan_status = Notice::quiet(
                    Severity::Success,
                    format!("Connected: local {local_addr}, remote {remote_addr}"),
                );
                self.lan_room = "LAN room connected".to_owned();
                self.post_ok(format!("LAN game connected: {}", rom.display()));
            }
            Err(error) => {
                self.lan_stats = None;
                self.lan_pace = None;
                self.lan_status =
                    Notice::quiet(Severity::Error, format!("Connection check failed: {error}"));
                self.lan_room = "LAN room offline".to_owned();
                self.post_error(format!("LAN game failed: {error}"));
            }
        }
    }
}
