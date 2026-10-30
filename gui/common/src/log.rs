//! # Note
//! Using `tracing` causes significant emulator latency, so it is not used.
//! This module provides a simple custom logger instead.
use std::{
    fs::{File, create_dir_all},
    io::{BufWriter, Write},
    sync::Mutex,
};

use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

/// Targets are the core crates' module paths (`ds_cpu::...`,
/// `ds_cartridge::...`, `ds_peripherals::wifi::...`).
///
/// # Examples
///
/// ```ignore
/// log::warn!(target: "ds_cpu", "hello {world}");
/// log::warn!(target: "ds_cartridge", "hello {world}");
/// ```
pub fn setup_logging() -> Result<(), SetLoggerError> {
    setup_logging_in(std::path::Path::new("logs"))
}

/// Installs the logger, writing its files into `dir`.
///
/// Takes the directory so each emulator instance can log into its own
/// `instances/instance<N>/logs`. The logger itself is process-global — only one
/// can be installed — so with several instances in one process the *first* to
/// call this wins and the others share its files. That is deliberate: the
/// alternative is interleaved writes to one file from several threads, which is
/// worse than a single well-defined destination.
pub fn setup_logging_in(dir: &std::path::Path) -> Result<(), SetLoggerError> {
    create_dir_all(dir).ok();
    let open =
        |name: &str| File::create(dir.join(name)).ok().map(|f| Mutex::new(BufWriter::new(f)));

    let logger =
        Logger { cpu: open("cpu.log"), wifi: open("wifi.log"), savedata: open("savedata.log") };

    log::set_boxed_logger(Box::new(logger))?;
    // NOTE: `LevelFilter::Trace` causes severe emulator slowdown.
    // Even when the logger discards messages, log records are generated and
    // routed through the logger for every trace call.
    // log::set_max_level(LevelFilter::Warn);
    log::set_max_level(LevelFilter::Info);

    Ok(())
}

struct Logger {
    cpu: Option<Mutex<BufWriter<File>>>,
    wifi: Option<Mutex<BufWriter<File>>>,
    savedata: Option<Mutex<BufWriter<File>>>,
}

impl Logger {
    #[inline(always)]
    fn write(writer: &Mutex<BufWriter<File>>, record: &Record) {
        let mut writer = writer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

        let file = record.file().unwrap_or("<unknown>");
        let line = record.line().unwrap_or(0);

        let _ = writeln!(writer, "[{}] {}:{} {}", record.level(), file, line, record.args());
    }

    #[inline(always)]
    fn stderr(record: &Record) {
        // tracing like
        let level_color = match record.level() {
            Level::Error => color::COLOR_RED,
            Level::Warn => color::COLOR_YELLOW,
            Level::Info => color::COLOR_GREEN,
            Level::Debug => color::COLOR_BLUE,
            Level::Trace => color::COLOR_MAGENTA,
        };

        let dim = "\x1b[2m";
        let reset = "\x1b[0m";

        let Some((group_name, target_color)) = color::target_group(record.target()) else {
            return; // skip unregistered name target(to fast emulate)
        };
        let file = record.file().unwrap_or("<unknown>");
        let line = record.line().unwrap_or(0);
        let level = record.level();
        let args = record.args();

        eprintln!(
            "[{target_color}{group_name}{reset}] {level_color}{level}{reset} \
            {dim}{file}:{line}{reset}: {level_color}{args}{reset}"
        );
    }
}

impl Log for Logger {
    #[inline(always)]
    fn enabled(&self, _: &Metadata) -> bool {
        true
    }

    #[inline(always)]
    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let target = record.target();
        let file = if target.starts_with("ds_cpu") || target.starts_with("ds_emu::cpu") {
            &self.cpu
        } else if target.starts_with("ds_peripherals::wifi") || target.starts_with("ds_net") {
            &self.wifi
        } else if target.starts_with("ds_cartridge") {
            &self.savedata
        } else {
            &None
        };
        if let Some(writer) = file {
            Self::write(writer, record);
        }

        Self::stderr(record);
    }

    fn flush(&self) {
        for writer in [&self.cpu, &self.wifi, &self.savedata].into_iter().flatten() {
            let _ = writer.lock().unwrap_or_else(std::sync::PoisonError::into_inner).flush();
        }
    }
}

#[allow(unused)]
mod color {
    pub(super) const COLOR_RED: &str = "\x1b[31m";
    pub(super) const COLOR_YELLOW: &str = "\x1b[33m";
    pub(super) const COLOR_GREEN: &str = "\x1b[32m";
    pub(super) const COLOR_BLUE: &str = "\x1b[34m";
    pub(super) const COLOR_MAGENTA: &str = "\x1b[35m";
    pub(super) const COLOR_CYAN: &str = "\x1b[36m";
    pub(super) const COLOR_WHITE: &str = "\x1b[37m";

    pub(super) const COLOR_RESET: &str = "\x1b[0m";
    pub(super) const COLOR_DIM: &str = "\x1b[2m";

    #[inline]
    pub(super) fn target_group(target: &str) -> Option<(&str, &'static str)> {
        Some(match target {
            t if t.starts_with("ds_cpu") || t.starts_with("ds_emu::cpu") => ("CPU", COLOR_CYAN),
            t if t.starts_with("ds_emu::emulator::dma") || t.starts_with("ds_emu::dma") => {
                ("DMA", COLOR_BLUE)
            }
            t if t.starts_with("ds_emu") || t.starts_with("ds_bios") => ("NDS", COLOR_BLUE),
            t if t.starts_with("ds_gpu") => ("GPU", COLOR_GREEN),
            t if t.starts_with("ds_cartridge") => ("SAVEDATA", COLOR_YELLOW),
            t if t.starts_with("ds_peripherals::wifi") || t.starts_with("ds_net") => {
                ("WIFI", COLOR_MAGENTA)
            }
            t if t.starts_with("ds_peripherals") || t.starts_with("ds_spu") => ("IO", COLOR_WHITE),
            _ => return None,
        })
    }
}
