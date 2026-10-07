//! LAN mode: the emulated wireless of two machines carried over UDP.
//!
//! # Flow
//!
//! ```text
//!  System ▸ Multiplayer ▸ Host / Join LAN game
//!    start_lan(host)
//!      1. remember the running cart, then unload it
//!      2. worker thread: LanHost::accept (wait for a guest)
//!                     or LanGuest::connect (say hello to the host)
//!      3. status: "waiting for a LAN guest on ..." / "connecting to ..."
//!  every repaint: poll_lan
//!      4. handshake done → boot the same cart with the link as its wifi
//!                        → keep the link's stats (Wireless pane) and its
//!                          pace (frame-rate limit, see crate::lan::LinkPace)
//!         failed         → report, back to "offline"
//! ```
//!
//! The transport itself — measured reply budget, redundant replies, pacing —
//! lives in [`crate::lan`]. Compare [`super::remote_session`], which answers
//! the same wish (play over a network) by streaming the picture instead.

use super::*;

/// The LAN port used when the address box gives none.
const LAN_PORT: u16 = 7064;

impl MelonEgui {
    /// What the live LAN link is doing, for the Wireless pane.
    #[must_use]
    pub fn lan_stats(&self) -> Option<crate::lan::LinkStats> {
        self.lan_stats.as_ref().map(|read| read())
    }

    /// Start hosting (`host`) or joining a LAN game, without blocking the UI.
    pub(crate) fn start_lan(&mut self, host: bool) {
        if self.lan_pending.is_some() {
            return self.post_warn(self.i18n().s(K::LanAlreadyPending));
        }
        let Some(rom) = self.emu.as_ref().map(|emu| emu.rom_path.clone()) else {
            return self.post_warn(self.i18n().s(K::LoadCartFirst));
        };
        // Checked here rather than only in the worker, so the complaint is in
        // the UI's language and comes before the running cart is unloaded.
        let typed = if host { &self.lan_bind_address } else { &self.lan_guest_address };
        if parse_lan_address(typed, LAN_PORT).is_err() {
            return self.post_error(self.i18n().f(K::InvalidAddress, &[typed]));
        }
        self.unload_cart();
        self.lan_rom = Some(rom);

        let (bind, address) = (self.lan_bind_address.clone(), self.lan_guest_address.clone());
        // Read once: a link keeps the tuning it started with.
        let tuning = self.lan_tuning;
        let (name, job_bind, job_address) = (
            if host { "melon-egui-lan-host" } else { "melon-egui-lan-guest" },
            bind.clone(),
            address.clone(),
        );
        let spawned = worker::spawn(name, move || {
            if host { lan_host(&job_bind, tuning) } else { lan_guest(&job_address, tuning) }
        });
        match spawned {
            Ok(receiver) => self.lan_pending = Some(receiver),
            Err(error) => {
                self.lan_rom = None;
                return self.post_error(self.i18n().f(K::LanCannotStart, &[&error]));
            }
        }
        // Saved on the attempt, so a retry does not mean typing it again.
        self.persist();
        // The one field, not `self.i18n()`: `lan_room` is assigned below.
        let tr = self.translations.get(self.language);
        self.lan_room = tr.s(if host { K::LanRoomHosting } else { K::LanRoomJoining });
        let (status, message) = if host {
            (tr.f(K::LanCheckingHost, &[&bind]), tr.f(K::LanWaitingGuest, &[&bind]))
        } else {
            (tr.f(K::LanCheckingGuest, &[&address]), tr.f(K::LanConnectingHost, &[&address]))
        };
        self.lan_status = Notice::quiet(Severity::Info, status);
        self.post(message);
    }

    /// Forget the LAN link the last console was on.
    ///
    /// Both handles must go: `lan_pace` would otherwise freeze at the old
    /// link's rate (a 100 ms link left the *next* cart running at ~10 fps), and
    /// `lan_stats` holds the transport alive, its threads pinging a peer that
    /// has gone.
    pub(crate) fn drop_link(&mut self) {
        self.lan_stats = None;
        self.lan_pace = None;
    }

    /// Finish a connection the worker established, and boot the cart on it.
    pub(crate) fn poll_lan(&mut self) {
        let Some(result) = worker::take(&mut self.lan_pending) else { return };
        let Some(result) = result else {
            return self.post_error(self.i18n().s(K::LanWorkerStopped));
        };
        let Some(rom) = self.lan_rom.take() else {
            return self.post_warn(self.i18n().s(K::LanNoCart));
        };
        let booted = result.and_then(|link| {
            let LanConnection { host, stats, pace, local_addr, remote_addr } = link;
            Emu::boot_lan(&rom, self.save_dir.as_ref(), self.state_dir.as_ref(), host)
                .map(|emu| (emu, stats, pace, local_addr, remote_addr))
        });
        match booted {
            Ok((emu, stats, pace, local_addr, remote_addr)) => {
                self.emu = Some(emu);
                self.lan_stats = Some(stats);
                self.lan_pace = Some(pace);
                self.reload_cheats(&rom);
                self.resume_fresh();
                let status = self.i18n().f(K::LanConnectedStatus, &[&local_addr, &remote_addr]);
                self.lan_status = Notice::quiet(Severity::Success, status);
                self.lan_room = self.i18n().s(K::LanRoomConnected);
                self.post_ok(self.i18n().f(K::LanGameConnected, &[&rom.display()]));
            }
            Err(error) => {
                self.drop_link();
                let status = self.i18n().f(K::LanCheckFailed, &[&error]);
                self.lan_status = Notice::quiet(Severity::Error, status);
                self.lan_room = self.i18n().s(K::LanRoomOffline);
                self.post_error(self.i18n().f(K::LanGameFailed, &[&error]));
            }
        }
    }
}

/// A link whose handshake finished on the worker thread.
pub(crate) struct LanConnection {
    /// The console's wireless back end (the link itself).
    host: Box<dyn melonds::Host>,
    local_addr: String,
    remote_addr: String,
    /// Reads the live counters, for the Wireless pane.
    stats: Box<dyn Fn() -> crate::lan::LinkStats + Send>,
    /// The frame rate the link sustains, for the pacing loop.
    pace: crate::lan::LinkPace,
}

/// Worker body for the host: bind, wait for one guest. Blocks.
fn lan_host(bind: &str, tuning: crate::lan::Tuning) -> Result<LanConnection, String> {
    let addr = parse_lan_address(bind, LAN_PORT)?;
    crate::lan::LanHost::accept(addr, tuning)
        .and_then(|link| {
            Ok(LanConnection {
                local_addr: link.local_addr()?.to_string(),
                remote_addr: link.remote_addr().to_string(),
                stats: Box::new(link.stats_reader()),
                pace: link.pace(),
                host: Box::new(link),
            })
        })
        .map_err(|e| e.to_string())
}

/// Worker body for the guest: say hello to the host until it answers. Blocks.
fn lan_guest(address: &str, tuning: crate::lan::Tuning) -> Result<LanConnection, String> {
    let remote = parse_lan_address(address, LAN_PORT)?;
    // Any local port: the host answers wherever the hello came from.
    let local = std::net::SocketAddr::from(([0, 0, 0, 0], 0));
    crate::lan::LanGuest::connect(local, remote, tuning)
        .and_then(|link| {
            Ok(LanConnection {
                local_addr: link.local_addr()?.to_string(),
                remote_addr: remote.to_string(),
                stats: Box::new(link.stats_reader()),
                pace: link.pace(),
                host: Box::new(link),
            })
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Emu, lan_guest, lan_host};

    /// Both worker bodies yield a link a console boots and runs on, and the
    /// stats reader stays usable beside it. Needs `MELON_TEST_ROM`.
    #[test]
    fn a_console_boots_on_each_end_of_a_loopback_link() {
        let Some(rom) = std::env::var_os("MELON_TEST_ROM")
            .map(std::path::PathBuf::from)
            .filter(|rom| rom.is_file())
        else {
            return;
        };
        let dir = std::env::temp_dir().join("melon_egui-lan-boot-test");
        std::fs::create_dir_all(&dir).unwrap();

        let tuning = crate::lan::Tuning::default();
        let host = std::thread::spawn(move || lan_host("127.0.0.1:47064", tuning));
        std::thread::sleep(Duration::from_millis(100));
        let guest = lan_guest("127.0.0.1:47064", tuning).expect("the guest connects");
        let host = host.join().unwrap().expect("the host accepts");
        assert_eq!(host.local_addr, "127.0.0.1:47064");
        assert_eq!(guest.remote_addr, "127.0.0.1:47064");

        for link in [host, guest] {
            let mut emu = Emu::boot_lan(&rom, Some(&dir), None, link.host).expect("boots");
            for _ in 0..30 {
                emu.run_frame_checked().expect("keeps running");
            }
            assert!((link.stats)().rtt_ms >= 0.0);
            assert!(link.pace.frame_rate() > 0.0);
        }
    }
}
