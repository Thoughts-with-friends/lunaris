//! Where this front end's and the core's diagnostics go.
//!
//! `melonds-rs` routes every melonDS `Log(...)` line through the `log` crate,
//! and a program with no logger installed drops all of them. So one is
//! installed here, writing to three places at once:
//!
//! * `logs/melon_egui.log` — every record at [`FILE_LEVEL`] or above.
//! * `logs/core.log` — the same, but only the emulator core's records, so a
//!   wireless or ARM fault can be read without the UI's chatter around it.
//! * stderr — coloured by level, at the level [`install`] is given.
//!
//! A short tail is also kept in memory for the crash report, and that one keeps
//! *everything*: a stopped console is explained by what was said just before
//! it, which is exactly what a level filter throws away. See [`recent`].
//!
//! # Why the levels differ
//!
//! Every record is generated and routed here, because the ring wants them all.
//! Writing them all to disk is the part that costs: two syscalls per line on
//! the emulation thread is enough to be felt, which is the slowdown
//! `lunaris_gui_common::log` documents. So the files stop at [`FILE_LEVEL`] and
//! flush only when something went wrong.

use std::{
    collections::VecDeque,
    fs::{File, create_dir_all},
    io::{BufWriter, Write},
    path::Path,
    sync::{Mutex, OnceLock},
};

use log::{Level, LevelFilter, Metadata, Record};

/// How many recent lines are kept for the crash report.
const HISTORY: usize = 200;

/// Records whose target starts with this are the emulator core's.
const CORE_TARGET: &str = "melonds";

/// The lowest level written to the log files.
///
/// `Debug` and `Trace` still reach the crash-report ring; they just do not pay
/// for a write and a flush each while a console is running.
const FILE_LEVEL: LevelFilter = LevelFilter::Info;

/// Install the logger, writing its files into `dir` and printing at `print`
/// or above.
///
/// `print` is `None` for an ordinary run, which then follows `RUST_LOG` and
/// defaults to warnings and errors. The headless harnesses pass `Info`: their
/// report *is* their output, so it must reach the terminal without the user
/// having to know about an environment variable.
///
/// Called once at startup; a second call is a no-op rather than an error,
/// because the two harnesses and the window all start here.
pub fn install(dir: &Path, print: Option<LevelFilter>) {
    static LOGGER: OnceLock<Logger> = OnceLock::new();

    let logger = LOGGER.get_or_init(|| Logger::new(dir, print));
    if log::set_logger(logger).is_ok() {
        // Everything reaches [`Logger::log`]; what is *printed* is decided
        // there, so the log files and the crash report keep the lines a quiet
        // session would have thrown away.
        log::set_max_level(LevelFilter::Trace);
    }
}

/// The kept lines, oldest first.
pub fn recent() -> Vec<String> {
    history().lock().map(|buf| buf.iter().cloned().collect()).unwrap_or_default()
}

/// The in-memory tail shared by every logger instance.
fn history() -> &'static Mutex<VecDeque<String>> {
    static BUFFER: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    BUFFER.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// A log file, or nothing if it could not be opened.
type LogFile = Option<Mutex<BufWriter<File>>>;

struct Logger {
    /// Every record.
    all: LogFile,
    /// Only the emulator core's records.
    core: LogFile,
    /// The level at or above which a record also reaches stderr.
    print: LevelFilter,
}

impl Logger {
    fn new(dir: &Path, print: Option<LevelFilter>) -> Self {
        create_dir_all(dir).ok();
        let open =
            |name: &str| File::create(dir.join(name)).ok().map(|f| Mutex::new(BufWriter::new(f)));

        Self {
            all: open("melon_egui.log"),
            core: open("core.log"),
            print: print.unwrap_or_else(print_level_from_env),
        }
    }

    /// Append `line`, flushing only if it is worth losing the buffer for.
    ///
    /// A warning or an error is very often the last thing said before the
    /// process stops, and a buffered one is a buffer nobody reads. Everything
    /// else rides the buffer out, which is what keeps this off the emulation
    /// thread's critical path.
    fn write(file: &LogFile, line: &str, urgent: bool) {
        if let Some(writer) = file
            && let Ok(mut writer) = writer.lock()
        {
            let _ = writeln!(writer, "{line}");
            if urgent {
                let _ = writer.flush();
            }
        }
    }

    /// Print with the level in colour, as lunaris's own logger does.
    fn print(record: &Record<'_>, line: &str) {
        const RESET: &str = "\x1b[0m";
        let colour = match record.level() {
            Level::Error => "\x1b[31m",
            Level::Warn => "\x1b[33m",
            Level::Info => "\x1b[32m",
            Level::Debug => "\x1b[34m",
            Level::Trace => "\x1b[35m",
        };
        // Deliberately the one place this crate writes to stderr; every other
        // site goes through the `log` macros.
        eprintln!("{colour}{line}{RESET}");
    }
}

impl log::Log for Logger {
    /// Always true: every record is kept for the files and the crash report,
    /// and [`Self::log`] decides separately whether to print it.
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &Record<'_>) {
        // The core's lines often end in a newline of their own, and a doubled
        // blank line reads like a gap in the log.
        let message = record.args().to_string();
        let line =
            format!("[{} {}] {}", level_name(record.level()), record.target(), message.trim_end());

        if record.level() <= FILE_LEVEL {
            let urgent = record.level() <= Level::Warn;
            Self::write(&self.all, &line, urgent);
            if record.target().starts_with(CORE_TARGET) {
                Self::write(&self.core, &line, urgent);
            }
        }
        if record.level() <= self.print {
            Self::print(record, &line);
        }
        // The ring keeps everything, whatever the two filters above let past:
        // it is the only record of what a console said before it stopped.
        if let Ok(mut buf) = history().lock() {
            buf.push_back(line);
            while buf.len() > HISTORY {
                buf.pop_front();
            }
        }
    }

    fn flush(&self) {
        for file in [&self.all, &self.core] {
            if let Some(writer) = file
                && let Ok(mut writer) = writer.lock()
            {
                let _ = writer.flush();
            }
        }
    }
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

/// `RUST_LOG`, as far as this needs it: a bare level name. Anything else — a
/// per-target filter, an unparseable value — falls back to warnings and errors,
/// which is what a user who has not asked for logs wants to see. The log files
/// are unaffected; this only decides what interrupts a terminal.
fn print_level_from_env() -> LevelFilter {
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
