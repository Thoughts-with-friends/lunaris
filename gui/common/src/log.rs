//! # Note
//! Using `tracing` causes significant emulator latency, so it is not used.
//! This module provides a simple custom logger instead.
use std::{
    fs::{File, create_dir_all},
    io::{BufWriter, Write},
    sync::Mutex,
};

use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};
use nds_core::log;

pub fn setup_logging() -> Result<(), SetLoggerError> {
    create_dir_all("logs").ok();

    let logger = Logger {
        arm7: File::create("logs/arm7.log").ok().map(|f| Mutex::new(BufWriter::new(f))),
        arm9: File::create("logs/arm9.log").ok().map(|f| Mutex::new(BufWriter::new(f))),
        savedata: File::create("logs/savedata.log").ok().map(|f| Mutex::new(BufWriter::new(f))),
    };

    log::set_boxed_logger(Box::new(logger))?;
    log::set_max_level(LevelFilter::Trace);

    Ok(())
}

struct Logger {
    arm7: Option<Mutex<BufWriter<File>>>,
    arm9: Option<Mutex<BufWriter<File>>>,
    savedata: Option<Mutex<BufWriter<File>>>,
}

impl Logger {
    #[inline(always)]
    fn write(writer: &Mutex<BufWriter<File>>, record: &Record) {
        let mut writer = writer.lock().unwrap();

        let file = record.file().unwrap_or("<unknown>");
        let line = record.line().unwrap_or(0);

        let _ = writeln!(writer, "[{}] {}:{} {}", record.level(), file, line, record.args());
    }

    #[inline(always)]
    fn stderr(record: &Record) {
        if record.level() > Level::Warn {
            return;
        }

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

        let (group_name, target_color) = color::target_group(record.target());
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

        match record.target() {
            "nds_core::arm7" => {
                if let Some(writer) = &self.arm7 {
                    Self::write(writer, record);
                }
            }
            "nds_core::arm9" => {
                if let Some(writer) = &self.arm9 {
                    Self::write(writer, record);
                }
            }
            "nds_core::savedata" => {
                if let Some(writer) = &self.savedata {
                    Self::write(writer, record);
                }
            }

            _ => {}
        }

        Self::stderr(record);
    }

    fn flush(&self) {
        if let Some(writer) = &self.arm7 {
            let _ = writer.lock().unwrap().flush();
        }

        if let Some(writer) = &self.arm9 {
            let _ = writer.lock().unwrap().flush();
        }

        if let Some(writer) = &self.savedata {
            let _ = writer.lock().unwrap().flush();
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
    pub(super) fn target_group(target: &str) -> (&str, &'static str) {
        match target {
            t if t.starts_with("nds_core::arm7") => ("ARM7", COLOR_CYAN),
            t if t.starts_with("nds_core::arm9") => ("ARM9", COLOR_MAGENTA),
            t if t.starts_with("nds_core::gpu") => ("GPU", COLOR_GREEN),
            t if t.starts_with("nds_core::dma") => ("DMA", COLOR_BLUE),
            t if t.starts_with("nds_core::savedata") => ("SAVEDATA", COLOR_YELLOW),
            _ => (target, "\x1b[37m"),
        }
    }
}
