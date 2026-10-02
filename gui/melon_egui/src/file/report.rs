//! The crash report a stopped console leaves behind.
//!
//! ```text
//!  write_crash_report(who, note)
//!   1. header            who stopped and why, cart, frame counts
//!   2. airwaves          per console: on the air?, frames sent/received
//!   3. traffic           the last 40 wireless frames
//!   4. core log          the last lines the core logged (kept in memory)
//!   5. → instance1/last-stop.txt, and the "Why the console stopped" pane
//! ```
//!
//! Written to a file because the usual way to run this front end is the
//! executable, which on Windows has no console to print to.

use std::fmt::Write as _;

use crate::app::*;

impl MelonEgui {
    /// Gather everything that might explain a stopped console, show it in the
    /// Crash pane, and write it to `instance1/last-stop.txt`.
    pub(crate) fn write_crash_report(&mut self, who: &str, note: &str) {
        let mut report = self.report_header(who, note);
        self.report_airwaves(&mut report);
        self.report_traffic(&mut report);
        report.push_str("\n-- the core's own last words -------------------------\n");
        for line in crate::logger::recent() {
            report.push_str(&line);
            report.push('\n');
        }
        self.save_report(report);
    }

    /// Who stopped and why, which cart, and how far each console had got.
    fn report_header(&mut self, who: &str, note: &str) -> String {
        let mut report = format!("melon_egui: {who} {note}\n");
        if let Some(emu) = &mut self.emu {
            let _ = write!(
                report,
                "cart: {} [{}]\nframes run: console 0 = {}",
                emu.info.title,
                emu.info.gamecode,
                emu.nds.frame_count()
            );
        }
        if let Some(guest) = &self.guest {
            let _ = write!(report, ", second instance = {}", guest.frame_count());
        }
        report.push('\n');
        report
    }

    /// Each console's wireless counters: local play failing shows up here as
    /// one side sending and nothing coming back.
    fn report_airwaves(&self, report: &mut String) {
        let connected = self.airwaves.connected();
        for (i, c) in self.airwaves.counters().iter().enumerate().take(2) {
            let _ = writeln!(
                report,
                "console {i}: {} | sent {}/{} cmd/reply, generic {}, ack {} | \
                 received cmd {}, reply {}, generic {} | stale replies {} | \
                 wifi clock {} | last reply mask {:04X}",
                if connected.get(i) == Some(&true) { "on the air" } else { "not on the air" },
                c.sent_cmd,
                c.sent_reply,
                c.sent_generic,
                c.sent_ack,
                c.recv_cmd,
                c.recv_reply,
                c.recv_generic,
                c.stale_replies,
                c.clock,
                c.last_reply_mask,
            );
        }
    }

    /// The last 40 frames on the airwaves.
    fn report_traffic(&self, report: &mut String) {
        report.push_str("\n-- the last of the wireless traffic ------------------\n");
        let log = self.airwaves.log();
        for event in log.iter().rev().take(40).rev() {
            let _ = writeln!(
                report,
                "console {} {} len={} ts={}",
                event.sender,
                event.kind.label(),
                event.len,
                event.timestamp
            );
        }
    }

    /// Write the report out and open the pane that shows it.
    fn save_report(&mut self, report: String) {
        let dir = crate::file::settings::config_dir();
        let path = dir.join("last-stop.txt");
        match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, &report)) {
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
