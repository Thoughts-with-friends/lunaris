//! Somewhere for the core's own diagnostics to go.
//!
//! `melonds-rs` routes every melonDS `Log(...)` line through the `log` crate,
//! and a program with no logger installed drops all of them. That is fine until
//! something goes wrong inside the core — a bad frame length from the wireless
//! stack, `EXCEPTION REGION NOT EXECUTABLE` from the ARM9 — at which point the
//! one message that explains the failure is the one nobody sees.
//!
//! So: a logger of about twenty lines rather than a dependency. Warnings and
//! errors are printed by default, since those are the ones worth interrupting
//! for; `RUST_LOG=debug` (or `info`, `trace`, `off`) turns the rest on for a
//! session that is chasing something.

use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};

use log::{Level, LevelFilter, Metadata, Record};

/// How many recent lines are kept for a crash report.
const HISTORY: usize = 200;

/// The last [`HISTORY`] lines, whatever the level filter lets through.
///
/// A stopped console is explained by what the core said just before it — and
/// that is exactly what is gone by the time anyone thinks to look. Keeping a
/// short tail costs nothing and turns "it stopped" into a report.
fn history() -> &'static Mutex<VecDeque<String>> {
    static HISTORY_BUF: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    HISTORY_BUF.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// The kept lines, oldest first.
pub fn recent() -> Vec<String> {
    history().lock().map(|buf| buf.iter().cloned().collect()).unwrap_or_default()
}

/// Prints to stderr, prefixed with the target so a core line is never mistaken
/// for one of this front end's own.
struct Stderr;

impl log::Log for Stderr {
    /// Always true: every line is kept for the crash report, and
    /// [`Self::log`] decides separately whether to print it. The cost is one
    /// formatted string per line the core emits, which even at `Trace` is a
    /// rounding error beside emulating two consoles.
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &Record<'_>) {
        let printed = record.level() <= print_level();
        // The core's lines already end in a newline of their own often enough
        // that trimming is worth it; a doubled blank line reads like a gap in
        // the log.
        let message = record.args().to_string();
        let line =
            format!("[{} {}] {}", level_name(record.level()), record.target(), message.trim_end());
        if printed {
            eprintln!("{line}");
        }
        if let Ok(mut buf) = history().lock() {
            buf.push_back(line);
            while buf.len() > HISTORY {
                buf.pop_front();
            }
        }
    }

    fn flush(&self) {}
}

const fn level_name(level: Level) -> &'static str {
    match level {
        Level::Error => "error",
        Level::Warn => "warn",
        Level::Info => "info",
        Level::Debug => "debug",
        Level::Trace => "trace",
    }
}

/// Install it. Called once at startup; a second call is a no-op rather than an
/// error, because the two headless harnesses and the window all start here.
pub fn install() {
    static LOGGER: Stderr = Stderr;
    if log::set_logger(&LOGGER).is_ok() {
        // The *filter* is applied when printing, not here: `log::max_level` is
        // what the `log!` macros check before they even format, and the crash
        // report wants the lines a quiet session would have thrown away.
        log::set_max_level(LevelFilter::Trace);
        PRINT_LEVEL.set(level_from_env()).ok();
    }
}

/// What actually reaches stderr; see [`install`].
static PRINT_LEVEL: OnceLock<LevelFilter> = OnceLock::new();

fn print_level() -> LevelFilter {
    *PRINT_LEVEL.get().unwrap_or(&LevelFilter::Warn)
}

/// `RUST_LOG`, as far as this needs it: a bare level name. Anything else — a
/// per-target filter, an unparseable value — falls back to warnings and errors,
/// which is what a user who has not asked for logs wants to see.
fn level_from_env() -> LevelFilter {
    let Ok(value) = std::env::var("RUST_LOG") else {
        return LevelFilter::Warn;
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "off" => LevelFilter::Off,
        "error" => LevelFilter::Error,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Warn,
    }
}
