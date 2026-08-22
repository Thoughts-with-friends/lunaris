//! The console's real-time clock, and the fixed one a repeatable run needs.

use super::*;

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
    pub(crate) const fn from_parts(parts: (i32, i32, i32, i32, i32, i32)) -> Self {
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
pub(crate) const FIXED_RTC: (i32, i32, i32, i32, i32, i32) = (2026, 1, 1, 12, 0, 0);

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

pub(crate) fn deterministic_rtc() -> bool {
    DETERMINISTIC_RTC.load(std::sync::atomic::Ordering::Relaxed)
}

/// The current UTC date and time as `(year, month, day, hour, minute, second)`.
///
/// The DS RTC is set once at boot and runs on emulated time from there. Local
/// time would be preferable for carts with a day/night cycle, but the offset is
/// not reachable without pulling in a date-time crate, so UTC it is.
pub(crate) fn utc_now() -> (i32, i32, i32, i32, i32, i32) {
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
pub(crate) fn civil_from_days(days: i64) -> (i32, i32, i32) {
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
