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

/// Why melonDS stopped a console, as `Platform::StopReason`.
///
/// The core hands one of these to the host on its way out — the only account
/// of *why* a console stopped, since `run_frame` reports the fact alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StopReason {
    /// No reason given.
    Unknown,
    /// Someone outside the console asked it to stop.
    External,
    /// The cart asked for GBA mode, which melonDS does not emulate.
    GbaModeNotSupported,
    /// The ARM9 took an exception with its vectors in memory the protection
    /// unit will not execute: a crash inside the emulated console, and the
    /// interesting case.
    BadExceptionRegion,
    /// The console shut itself down — the cart wrote the power-management
    /// chip's shutdown bit. Not a fault.
    PowerOff,
    /// A reason this build does not know about.
    Other(i32),
}

impl StopReason {
    /// melonDS `Platform::StopReason`, in its declaration order (see
    /// `Platform.h`: Unknown, External, GBAModeNotSupported,
    /// BadExceptionRegion, PowerOff).
    const fn from_core(reason: i32) -> Self {
        match reason {
            0 => Self::Unknown,
            1 => Self::External,
            2 => Self::GbaModeNotSupported,
            3 => Self::BadExceptionRegion,
            4 => Self::PowerOff,
            other => Self::Other(other),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "stopped for no stated reason",
            Self::External => "was stopped from outside",
            Self::GbaModeNotSupported => "asked for GBA mode, which is not emulated",
            Self::BadExceptionRegion => {
                "crashed: the ARM9 took an exception with its vectors in non-executable memory"
            }
            Self::PowerOff => "powered itself off",
            Self::Other(_) => "stopped for a reason this build does not know",
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
    /// The reason the core last gave for stopping, filled in from the host
    /// callback while `run_frame` was running.
    stop: Arc<Mutex<Option<StopReason>>>,
    /// This console's seat on the airwaves and the instance number that goes
    /// with it, kept so that a reboot takes the same seat rather than dropping
    /// off the air.
    seat: Option<(u32, crate::mp::Client)>,
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

    /// As [`Emu::boot_with`], but joined to shared airwaves as console
    /// `instance_id`.
    ///
    /// `instance_id` also uniquifies the generated firmware's MAC address, the
    /// same way melonDS's frontend does, so two consoles on one medium are not
    /// indistinguishable to each other.
    pub fn boot_mp(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
        instance_id: u32,
        mp: crate::mp::Client,
    ) -> Result<Self, String> {
        Self::boot_inner(rom_path, save_dir, state_dir, instance_id, Some(mp))
    }

    /// As [`Emu::boot`], but with the save and savestate directories overridden;
    /// `None` for either means "beside the ROM".
    pub fn boot_with(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
    ) -> Result<Self, String> {
        Self::boot_inner(rom_path, save_dir, state_dir, 0, None)
    }

    fn boot_inner(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
        instance_id: u32,
        mp: Option<crate::mp::Client>,
    ) -> Result<Self, String> {
        let rom = std::fs::read(rom_path).map_err(|e| format!("cannot read ROM: {e}"))?;
        let save_path = crate::config::Settings::redirect(save_dir, rom_path, "sav");
        let state_dir = state_dir.cloned();
        let save = std::fs::read(&save_path).ok();

        let saves = Arc::new(SaveSink { path: save_path, pending: Mutex::new(None) });
        let stop = Arc::new(Mutex::new(None));
        // The seat is cloned rather than moved: the console's `Host` owns one
        // handle to the airwaves, and the front end keeps another so a reboot
        // (importing a save) can take the same seat again.
        let seat = mp.clone().map(|mp| (instance_id, mp));
        let host = Box::new(HostBridge { saves: Arc::clone(&saves), stop: Arc::clone(&stop), mp });

        let mut nds = Nds::new(&rom, save.as_deref(), instance_id, host)
            .map_err(|e| format!("cart rejected: {e}"))?;
        let (y, mo, d, h, mi, s) = if deterministic_rtc() { FIXED_RTC } else { utc_now() };
        nds.set_rtc(y, mo, d, h, mi, s);
        nds.boot();

        Ok(Self {
            nds,
            rom_path: rom_path.to_owned(),
            info: CartInfo::parse(&rom),
            saves,
            state_dir,
            stop,
            seat,
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

    /// Why the core stopped, if it has, taken out on the way past.
    ///
    /// Reported with both CPUs' program counters, since the reason alone does
    /// not say *where*: an ARM9 crash during wireless play, for instance, is
    /// usually a fault the ARM7's wifi handling led it into.
    pub fn stop_reason(&mut self) -> Option<String> {
        let reason = self.stop.lock().unwrap().take()?;
        Some(format!(
            "{} (ARM9 pc={:08X}, ARM7 pc={:08X})",
            reason.label(),
            self.nds.pc(),
            self.nds.arm7_pc()
        ))
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

    /// Apply the Video settings, returning the renderer the core actually
    /// installed.
    ///
    /// That can differ from the one asked for: melonDS falls back to the
    /// software renderer rather than leave a console unable to draw, so a
    /// machine whose driver cannot compile its shaders answers
    /// [`melonds::Renderer::Software`] here. Asking for an OpenGL renderer
    /// requires a current GL context on this thread, both here and on every
    /// subsequent frame.
    pub fn set_render_settings(&mut self, settings: melonds::RenderSettings) -> melonds::Renderer {
        self.nds.set_render_settings(settings)
    }

    /// Where the OpenGL renderer left the picture, if it is the one in use.
    pub fn gl_output(&mut self) -> Option<melonds::GlOutput> {
        self.nds.gl_output()
    }

    /// Build one of a lazily-compiled renderer's shaders, reporting progress
    /// as `(done, total)` while any remain. Only the compute renderer has
    /// any; for the others this is `None` from the first call.
    pub fn gl_shader_compile_step(&mut self) -> Option<(u32, u32)> {
        self.nds.gl_shader_compile_step()
    }

    /// Read one screen of the OpenGL renderer's output back into host memory,
    /// BGRA8888 and top-down, at the internal resolution.
    ///
    /// The headless harnesses capture the software renderer's framebuffers
    /// directly; this is how they capture a picture that only ever existed in
    /// a texture.
    pub fn gl_read_output(&mut self, screen: u8, out: &mut [u32]) -> usize {
        self.nds.gl_read_output(screen, out)
    }

    /// Open or close the lid, as melonDS's Power management does. Closing it
    /// raises the lid IRQ, which is how a cart is told to sleep.
    pub fn set_lid_closed(&mut self, closed: bool) {
        self.nds.set_lid_closed(closed);
    }

    pub fn lid_closed(&mut self) -> bool {
        self.nds.lid_closed()
    }

    /// What the power-management chip reports about the battery: `true` for
    /// okay, `false` for the low level a cart warns about.
    pub fn set_battery_okay(&mut self, okay: bool) {
        self.nds.set_battery_okay(okay);
    }

    pub fn battery_okay(&mut self) -> bool {
        self.nds.battery_okay()
    }

    /// Turn framebuffer production on or off.
    ///
    /// Off, the console keeps running and keeps capturing to VRAM; only the
    /// framebuffer the front end reads goes stale.
    pub fn set_render(&mut self, enabled: bool) {
        self.nds.set_render(enabled);
    }

    /// Tell the core which screens anyone is looking at: bit 0 top, bit 1
    /// bottom. An engine whose screen is not shown does not compose it.
    pub fn set_displayed_screens(&mut self, mask: u8) {
        self.nds.set_displayed_screens(mask);
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
        // Rebooting must not cost this console its place on the airwaves: a
        // `Host` is fixed at construction, so a console rebooted without its
        // seat could never join one afterwards.
        let save_dir = self.saves.path.parent().map(Path::to_path_buf);
        let (instance_id, mp) = match self.seat.clone() {
            Some((id, mp)) => (id, Some(mp)),
            None => (0, None),
        };
        let reloaded = Self::boot_inner(
            &self.rom_path,
            save_dir.as_ref(),
            self.state_dir.as_ref(),
            instance_id,
            mp,
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
    /// Where `signal_stop` leaves what it was told, for the front end to read
    /// once `run_frame` has returned.
    stop: Arc<Mutex<Option<StopReason>>>,
    /// This console's place on the shared airwaves, when it has one. Without it
    /// every MP hook keeps the trait's default — an unlinked console.
    mp: Option<crate::mp::Client>,
}

impl melonds::Host for HostBridge {
    /// `data` is the whole backup image, so the offset/length hint of *which*
    /// bytes moved is not needed to keep the file correct — only to write less
    /// than all of it, which this front end does not bother with.
    fn write_save(&self, data: &[u8], _writeoffset: u32, _writelen: u32) {
        *self.saves.pending.lock().unwrap() = Some(data.to_vec());
    }

    /// melonDS is stopping this console. Recorded rather than acted on: the
    /// call arrives from inside `run_frame`, and the front end reads it out
    /// once that has returned — see [`Emu::stop_reason`].
    fn signal_stop(&self, reason: i32) {
        *self.stop.lock().unwrap() = Some(StopReason::from_core(reason));
    }

    // The MP hooks are pure forwarding: the airwaves are shared state, so the
    // interesting behaviour lives in `crate::mp` rather than here.

    fn mp_begin(&self) {
        if let Some(mp) = &self.mp {
            mp.mp_begin();
        }
    }

    fn mp_end(&self) {
        if let Some(mp) = &self.mp {
            mp.mp_end();
        }
    }

    fn mp_send_packet(&self, data: &[u8], timestamp: u64) -> i32 {
        self.mp.as_ref().map_or(data.len() as i32, |mp| mp.mp_send_packet(data, timestamp))
    }

    fn mp_send_cmd(&self, data: &[u8], timestamp: u64) -> i32 {
        self.mp.as_ref().map_or(data.len() as i32, |mp| mp.mp_send_cmd(data, timestamp))
    }

    fn mp_send_reply(&self, data: &[u8], timestamp: u64, aid: u16) -> i32 {
        self.mp.as_ref().map_or(data.len() as i32, |mp| mp.mp_send_reply(data, timestamp, aid))
    }

    fn mp_send_ack(&self, data: &[u8], timestamp: u64) -> i32 {
        self.mp.as_ref().map_or(data.len() as i32, |mp| mp.mp_send_ack(data, timestamp))
    }

    fn mp_recv_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        self.mp.as_ref().map_or(Some(0), |mp| mp.mp_recv_packet(data, now, timestamp))
    }

    fn mp_recv_host_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        self.mp.as_ref().and_then(|mp| mp.mp_recv_host_packet(data, now, timestamp))
    }

    fn mp_recv_replies(&self, data: &mut [u8], now: u64, timestamp: u64, aidmask: u16) -> u16 {
        self.mp.as_ref().map_or(0, |mp| mp.mp_recv_replies(data, now, timestamp, aidmask))
    }

    fn mp_clock(&self, now: u64) {
        if let Some(mp) = &self.mp {
            mp.mp_clock(now);
        }
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
    use std::sync::{Arc, Mutex};

    use melonds::Host;

    use super::{HostBridge, SaveSink, civil_from_days};
    use crate::mp::Airwaves;

    /// A bridge with the seat a console booted for local play gets.
    fn bridge(air: &Airwaves, instance: usize) -> HostBridge {
        HostBridge {
            saves: Arc::new(SaveSink {
                path: std::path::PathBuf::from("unused.sav"),
                pending: Mutex::new(None),
            }),
            stop: Arc::new(Mutex::new(None)),
            mp: (instance < usize::MAX).then(|| air.client(instance)),
        }
    }

    /// The bug local play failed on: a console booted without a seat has no
    /// way to join one later, because its `Host` is fixed when the core is
    /// constructed. Its frames used to vanish into the trait's defaults while
    /// the other console sat waiting for a host that could never be heard.
    #[test]
    fn a_console_with_a_seat_is_actually_on_the_air() {
        let air = Airwaves::new();
        let (host, guest) = (bridge(&air, 0), bridge(&air, 1));
        host.mp_begin();
        guest.mp_begin();

        host.mp_send_cmd(b"round", 1000);

        let mut buf = [0u8; 64];
        let mut ts = 0;
        assert_eq!(guest.mp_recv_host_packet(&mut buf, 0, &mut ts), Some(5));
        assert_eq!(&buf[..5], b"round");
        assert_eq!(air.counters()[0].sent_cmd, 1, "the console's CMD reached the medium");
    }

    #[test]
    fn a_console_without_a_seat_hears_nothing_and_is_heard_by_nobody() {
        let air = Airwaves::new();
        let guest = bridge(&air, 1);
        guest.mp_begin();
        // What `Emu::boot_with` builds: no seat at all.
        let seatless = HostBridge {
            saves: Arc::new(SaveSink {
                path: std::path::PathBuf::from("unused.sav"),
                pending: Mutex::new(None),
            }),
            stop: Arc::new(Mutex::new(None)),
            mp: None,
        };

        // The trait's defaults claim the send succeeded, which is exactly why
        // this was invisible: nothing reports an error.
        assert_eq!(seatless.mp_send_cmd(b"round", 1000), 5);
        let mut buf = [0u8; 64];
        let mut ts = 0;
        assert_eq!(guest.mp_recv_packet(&mut buf, 0, &mut ts), Some(0), "nothing arrived");
        assert_eq!(air.counters()[0].sent_cmd, 0);
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        // 2000-02-29: a leap day in a century year that is also a leap year.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_544), (2026, 4, 1));
    }
}
