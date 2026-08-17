//! Core ownership: booting a cart, and the [`melonds::Host`] the core calls
//! back into.
//!
//! Everything host-side that melonDS's Qt frontend would own — save
//! persistence, the clock, the airwaves — reaches the core through the `Host`
//! trait. This front end implements only what a single offline console needs:
//! backup memory on disk, and a real-time RTC. Wireless is deliberately left at
//! the trait's defaults (an unlinked console); linking two instances is a
//! separate job, see `docs/design/review_mp_local2.md`.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};

use melonds::Nds;

/// What the cart header says about itself, for the ROM info pane.
pub struct CartInfo {
    /// The 12-byte game title, trimmed of its padding.
    pub title: String,
    /// The 4-character game code, e.g. `IPKJ`.
    pub gamecode: String,
    /// The 2-character maker code, e.g. `01` for Nintendo.
    pub maker: String,
    /// The ROM's size on disk, in bytes.
    pub size: usize,
}

impl CartInfo {
    /// Read the fields at the start of the cart header (GBATEK, "DS Cartridge
    /// Header": title at 0, game code at 0Ch, maker code at 10h). A ROM too
    /// short to hold a header yields empty strings rather than failing, since
    /// the core has already accepted it by this point.
    fn parse(rom: &[u8]) -> Self {
        let field = |range: std::ops::Range<usize>| {
            rom.get(range)
                .map(|bytes| {
                    String::from_utf8_lossy(bytes).trim_end_matches(['\0', ' ']).to_owned()
                })
                .unwrap_or_default()
        };
        Self {
            title: field(0x00..0x0C),
            gamecode: field(0x0C..0x10),
            maker: field(0x10..0x12),
            size: rom.len(),
        }
    }
}

/// A booted cart, plus the host-side state that outlives any single frame.
pub struct Emu {
    pub nds: Nds,
    /// Where the ROM came from, for window titles and for deriving the save and
    /// savestate paths.
    pub rom_path: PathBuf,
    pub info: CartInfo,
    /// The other end of the [`SaveSink`] handed to the core, so the front end
    /// can flush pending backup memory on its own schedule.
    saves: Arc<SaveSink>,
    /// Where savestates go; `None` means beside the ROM.
    state_dir: Option<PathBuf>,
}

impl Emu {
    /// Read `rom_path`, restore its backup memory if a `.sav` sits beside it,
    /// and direct-boot the cart.
    ///
    /// No BIOS or firmware files are needed: `melonds-sys`'s shim boots
    /// FreeBIOS with generated firmware.
    pub fn boot(rom_path: &Path) -> Result<Self, String> {
        Self::boot_with(rom_path, None, None)
    }

    /// As [`Emu::boot`], but with the save and savestate directories overridden;
    /// `None` for either means "beside the ROM".
    pub fn boot_with(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
    ) -> Result<Self, String> {
        let rom = std::fs::read(rom_path).map_err(|e| format!("cannot read ROM: {e}"))?;
        let save_path = crate::config::Settings::redirect(save_dir, rom_path, "sav");
        let state_dir = state_dir.cloned();
        let save = std::fs::read(&save_path).ok();

        let saves = Arc::new(SaveSink { path: save_path, pending: Mutex::new(None) });
        let host = Box::new(HostBridge { saves: Arc::clone(&saves) });

        let mut nds =
            Nds::new(&rom, save.as_deref(), 0, host).map_err(|e| format!("cart rejected: {e}"))?;
        let (y, mo, d, h, mi, s) = if deterministic_rtc() { FIXED_RTC } else { utc_now() };
        nds.set_rtc(y, mo, d, h, mi, s);
        nds.boot();

        Ok(Self {
            nds,
            rom_path: rom_path.to_owned(),
            info: CartInfo::parse(&rom),
            saves,
            state_dir,
        })
    }

    /// Write out backup memory if the core has changed it since the last call.
    ///
    /// The core reports every write as it happens, which is far more often than
    /// a file should be rewritten, so [`SaveSink`] only remembers the newest
    /// image and the front end drains it on a timer.
    pub fn flush_save(&self) {
        let Some(data) = self.saves.pending.lock().unwrap().take() else {
            return;
        };
        if let Err(e) = std::fs::write(&self.saves.path, &data) {
            eprintln!("melon_egui: failed to write {}: {e}", self.saves.path.display());
        }
    }

    /// Re-set the console's real-time clock.
    ///
    /// The RTC keeps counting in emulated time from whatever it is set to, so
    /// this takes effect immediately; carts that only read the clock at startup
    /// will not notice until they next look.
    pub fn set_clock(&mut self, clock: Clock) {
        self.nds.set_rtc(
            clock.year,
            clock.month,
            clock.day,
            clock.hour,
            clock.minute,
            clock.second,
        );
    }

    /// Hold or release white noise on the microphone.
    pub fn set_mic_static(&mut self, on: bool) {
        self.nds.set_mic_static(on);
    }

    /// Savestate path for one of the numbered slots, following melonDS's
    /// `<rom>.mlN` convention so the two front ends do not collide.
    pub fn state_path(&self, slot: u8) -> PathBuf {
        crate::config::Settings::redirect(
            self.state_dir.as_ref(),
            &self.rom_path,
            &format!("ml{slot}"),
        )
    }

    /// Replace the cart's backup memory with `data` and restart, which is the
    /// only way the core takes a foreign save: it reads backup memory once, at
    /// construction.
    pub fn import_save(&mut self, data: &[u8]) -> Result<(), String> {
        std::fs::write(&self.saves.path, data)
            .map_err(|e| format!("cannot write {}: {e}", self.saves.path.display()))?;
        // Drop whatever the core was about to write, so the old save cannot
        // land on top of the imported one.
        *self.saves.pending.lock().unwrap() = None;
        let reloaded = Self::boot_with(
            &self.rom_path,
            self.saves.path.parent().map(Path::to_path_buf).as_ref(),
            self.state_dir.as_ref(),
        )?;
        *self = reloaded;
        Ok(())
    }
}

impl Drop for Emu {
    /// A cart being unloaded is the last chance to persist its save.
    fn drop(&mut self) {
        self.flush_save();
    }
}

/// The newest backup-memory image the core has produced, waiting to be written.
struct SaveSink {
    path: PathBuf,
    pending: Mutex<Option<Vec<u8>>>,
}

/// The core's view of the host. Holds only what a callback needs; the front end
/// keeps its own [`Arc`] to the same sink.
struct HostBridge {
    saves: Arc<SaveSink>,
}

impl melonds::Host for HostBridge {
    /// `data` is the whole backup image, so the offset/length hint of *which*
    /// bytes moved is not needed to keep the file correct — only to write less
    /// than all of it, which this front end does not bother with.
    fn write_save(&self, data: &[u8], _writeoffset: u32, _writelen: u32) {
        *self.saves.pending.lock().unwrap() = Some(data.to_vec());
    }
}

/// A wall-clock date and time, as the DS's RTC takes it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Clock {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
}

impl Clock {
    const fn from_parts(parts: (i32, i32, i32, i32, i32, i32)) -> Self {
        let (year, month, day, hour, minute, second) = parts;
        Self { year, month, day, hour, minute, second }
    }
}

/// The current UTC date and time, for the Date and time dialog's "Now" button.
pub fn utc_clock() -> Clock {
    Clock::from_parts(if deterministic_rtc() { FIXED_RTC } else { utc_now() })
}

/// The clock a deterministic run boots with. Arbitrary, but fixed: what matters
/// is that two runs get the same one.
const FIXED_RTC: (i32, i32, i32, i32, i32, i32) = (2026, 1, 1, 12, 0, 0);

/// Whether new consoles get [`FIXED_RTC`] rather than the wall clock.
static DETERMINISTIC_RTC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Boot every console from here on with a fixed clock.
///
/// The RTC is the one input to an otherwise deterministic core that changes
/// between runs, and carts read it during their intros — so with the wall clock
/// two captures of "frame 2600" need not show the same thing. `--selftest` and
/// `--shot` exist to be compared against each other and against melonDS, so
/// they call this; an interactive session wants the real date and does not.
pub fn use_deterministic_rtc() {
    DETERMINISTIC_RTC.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn deterministic_rtc() -> bool {
    DETERMINISTIC_RTC.load(std::sync::atomic::Ordering::Relaxed)
}

/// The current UTC date and time as `(year, month, day, hour, minute, second)`.
///
/// The DS RTC is set once at boot and runs on emulated time from there. Local
/// time would be preferable for carts with a day/night cycle, but the offset is
/// not reachable without pulling in a date-time crate, so UTC it is.
fn utc_now() -> (i32, i32, i32, i32, i32, i32) {
    let secs =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let days = secs.div_euclid(86_400);
    let time_of_day = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    (
        y,
        mo,
        d,
        (time_of_day / 3600) as i32,
        (time_of_day % 3600 / 60) as i32,
        (time_of_day % 60) as i32,
    )
}

/// Days since the Unix epoch to a proleptic-Gregorian `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`, which is exact for the whole range this
/// is ever called with. Reimplemented rather than pulled in as a dependency:
/// setting a clock once at boot does not justify a date-time crate.
fn civil_from_days(days: i64) -> (i32, i32, i32) {
    // Shift the epoch to 0000-03-01 so leap days land at the end of the cycle.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era, [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // March-based month, [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y } as i32, m as i32, d as i32)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        // 2000-02-29: a leap day in a century year that is also a leap year.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_544), (2026, 4, 1));
    }
}
