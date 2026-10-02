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
            self.post_warn("a LAN connection is already being established");
            return;
        }
        let Some(rom) = self.emu.as_ref().map(|emu| emu.rom_path.clone()) else {
            self.post_warn("load a cart first");
            return;
        };
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
                self.post_error(format!("cannot start LAN connection: {error}"));
                return;
            }
        }
        // Saved on the attempt, so a retry does not mean typing it again.
        self.persist();
        self.lan_room = if host { "Hosting LAN room" } else { "Joining LAN room" }.to_owned();
        let (status, message) = if host {
            (
                format!("Checking: waiting for guest on {bind}"),
                format!("waiting for a LAN guest on {bind}"),
            )
        } else {
            (
                format!("Checking: connecting to {address}"),
                format!("connecting to LAN host {address}"),
            )
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
            self.post_error("LAN connection worker stopped unexpectedly");
            return;
        };
        let Some(rom) = self.lan_rom.take() else {
            self.post_warn("LAN connected, but no cart is loaded");
            return;
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
                self.lan_status = Notice::quiet(
                    Severity::Success,
                    format!("Connected: local {local_addr}, remote {remote_addr}"),
                );
                self.lan_room = "LAN room connected".to_owned();
                self.post_ok(format!("LAN game connected: {}", rom.display()));
            }
            Err(error) => {
                self.drop_link();
                self.lan_status =
                    Notice::quiet(Severity::Error, format!("Connection check failed: {error}"));
                self.lan_room = "LAN room offline".to_owned();
                self.post_error(format!("LAN game failed: {error}"));
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
        .map_err(|e| format!("LAN host failed: {e}"))
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
        .map_err(|e| format!("LAN guest failed: {e}"))
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
