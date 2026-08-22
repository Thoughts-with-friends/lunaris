//! The crash report a stopped console leaves behind.
//!
//! Written to a file because the usual way to run this front end is to launch
//! the executable, which on Windows has no console attached.

use crate::app::*;

impl MelonEgui {
    /// Gather everything that might explain a stopped console, show it in the
    /// Crash pane, and write it beside the executable.
    pub(crate) fn write_crash_report(&mut self, who: &str, note: &str) {
        let mut report = format!("melon_egui: {who} {note}\n");
        if let Some(emu) = &mut self.emu {
            report.push_str(&format!(
                "cart: {} [{}]\nframes run: console 0 = {}",
                emu.info.title,
                emu.info.gamecode,
                emu.nds.frame_count()
            ));
        }
        if let Some(guest) = &self.guest {
            report.push_str(&format!(", second instance = {}", guest.frame_count()));
        }
        report.push('\n');

        // Who was on the air, and what they had exchanged: local play failing
        // shows up here as one side sending and nothing coming back.
        let connected = self.airwaves.connected();
        for (i, counters) in self.airwaves.counters().iter().enumerate().take(2) {
            report.push_str(&format!(
                "console {i}: {} | sent {}/{} cmd/reply, generic {}, ack {} | \
                 received cmd {}, reply {}, generic {} | stale replies {} | \
                 wifi clock {} | last reply mask {:04X}\n",
                if connected.get(i) == Some(&true) { "on the air" } else { "not on the air" },
                counters.sent_cmd,
                counters.sent_reply,
                counters.sent_generic,
                counters.sent_ack,
                counters.recv_cmd,
                counters.recv_reply,
                counters.recv_generic,
                counters.stale_replies,
                counters.clock,
                counters.last_reply_mask,
            ));
        }

        report.push_str("\n-- the last of the wireless traffic ------------------\n");
        let log = self.airwaves.log();
        for event in log.iter().rev().take(40).rev() {
            report.push_str(&format!(
                "console {} {} len={} ts={}\n",
                event.sender,
                event.kind.label(),
                event.len,
                event.timestamp
            ));
        }

        report.push_str("\n-- the core's own last words -------------------------\n");
        for line in crate::logger::recent() {
            report.push_str(&line);
            report.push('\n');
        }

        let path = crate::file::settings::config_dir().join("last-stop.txt");
        match std::fs::create_dir_all(crate::file::settings::config_dir())
            .and_then(|()| std::fs::write(&path, &report))
        {
            Ok(()) => {
                log::info!("wrote {}", path.display());
                self.post_ok(format!("stop report written to {}", path.display()));
            }
            Err(e) => log::error!("could not write {}: {e}", path.display()),
        }
        self.crash_report = Some(report);
        if !self.panes.contains(&panes::Pane::Crash) {
            self.panes.push(panes::Pane::Crash);
        }
    }
}
