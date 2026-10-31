//! The `melonds` crate's API on the Lunaris core.
//!
//! melon_egui is written against `melonds` (melonDS through FFI). Built
//! with this crate in its place (`melonds = { package = "lunaris_melonds" }`
//! in `gui/egui`), the same front end — same windows, menus and panes —
//! drives this repository's core instead.
//!
//! ```text
//!  melon_egui sources ── melonds::Nds / Host / keys / Cheat ... ──┐
//!                                                                  v
//!  this crate:  Nds ──> ds_emu::Emulator (core/*)
//!               Host ──> HostLink (ds_peripherals MpLink) ──> Wi-Fi
//!               audio: 32.7 kHz SPU output resampled to 48 kHz
//!               saves: BackupMemory::take_dirty -> Host::write_save
//! ```
//!
//! What the core does not have answers the way melonDS answers a missing
//! capability: no OpenGL ([`gl_load`] is `false`, every renderer request
//! stays [`Renderer::Software`]), so the front end greys those out.
//! Savestates are the core's own format, not melonDS's.
use std::sync::{Arc, Mutex};

use ds_emu::Emulator;
use ds_peripherals::wifi::{MpLink, MpRecv};

mod audio;
use audio::Resampler;

/// Which back end this is; melon_egui shows it and disables what it lacks.
pub const BACKEND: &str = "lunaris";

/// Key bits for [`Nds::set_keys`] (active-high).
pub mod keys {
    pub const A: u32 = 1 << 0;
    pub const B: u32 = 1 << 1;
    pub const SELECT: u32 = 1 << 2;
    pub const START: u32 = 1 << 3;
    pub const RIGHT: u32 = 1 << 4;
    pub const LEFT: u32 = 1 << 5;
    pub const UP: u32 = 1 << 6;
    pub const DOWN: u32 = 1 << 7;
    pub const R: u32 = 1 << 8;
    pub const L: u32 = 1 << 9;
    pub const X: u32 = 1 << 10;
    pub const Y: u32 = 1 << 11;
}

pub const SCREEN_WIDTH: usize = 256;
pub const SCREEN_HEIGHT: usize = 192;

/// What the console calls back into (melonds `Host`, same methods and
/// defaults): save writes, stop, and the local-multiplayer medium.
#[allow(unused_variables)]
pub trait Host: Send {
    fn write_save(&self, data: &[u8], writeoffset: u32, writelen: u32) {}
    fn signal_stop(&self, reason: i32) {}

    fn mp_begin(&self) {}
    fn mp_end(&self) {}
    fn mp_send_packet(&self, data: &[u8], timestamp: u64) -> i32 {
        data.len() as i32
    }
    fn mp_recv_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        Some(0)
    }
    fn mp_send_cmd(&self, data: &[u8], timestamp: u64) -> i32 {
        data.len() as i32
    }
    fn mp_send_reply(&self, data: &[u8], timestamp: u64, aid: u16) -> i32 {
        data.len() as i32
    }
    fn mp_send_ack(&self, data: &[u8], timestamp: u64) -> i32 {
        data.len() as i32
    }
    fn mp_recv_host_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        None
    }
    fn mp_recv_replies(&self, data: &mut [u8], now: u64, timestamp: u64, aidmask: u16) -> u16 {
        0
    }
    fn mp_clock(&self, now: u64) {}
}

/// One Action Replay code.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Cheat {
    pub name: String,
    pub code: Vec<u32>,
    pub enabled: bool,
}

/// Rasteriser choice. The core has only its software renderer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Renderer {
    #[default]
    Software,
    OpenGl,
    Compute,
}

impl Renderer {
    pub const fn is_gl(self) -> bool {
        matches!(self, Self::OpenGl | Self::Compute)
    }
}

/// Video settings (melonds `RenderSettings`). Only `threaded` has an
/// effect here: it spreads the 3D rasteriser over worker threads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RenderSettings {
    pub renderer: Renderer,
    pub scale: u32,
    pub threaded: bool,
    pub hires_coordinates: bool,
    pub better_polygons: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            renderer: Renderer::Software,
            scale: 1,
            threaded: false,
            hires_coordinates: false,
            better_polygons: false,
        }
    }
}

/// Where an OpenGL renderer left its picture (never, here).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GlOutput {
    pub texture: u32,
    pub width: u32,
    pub height: u32,
}

/// The core has no OpenGL renderer: binding never succeeds.
pub fn gl_load(
    _loader: Option<unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_void>,
) -> bool {
    false
}

pub const fn gl_supports_compute() -> bool {
    false
}

pub const fn gl_info() -> Option<String> {
    None
}

#[derive(Debug)]
pub enum Error {
    BadRom,
    Savestate,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BadRom => "cart rejected (not a parseable NDS ROM)",
            Self::Savestate => "savestate failed",
        })
    }
}

impl std::error::Error for Error {}

/// The host's MP calls as the core's Wi-Fi link.
struct HostLink {
    host: Arc<Mutex<Box<dyn Host>>>,
    /// Wi-Fi clock of the last `clock` call: the `now` of receives.
    now: u64,
}

impl std::fmt::Debug for HostLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostLink").field("now", &self.now).finish()
    }
}

impl HostLink {
    fn with<R>(&self, f: impl FnOnce(&dyn Host) -> R) -> R {
        let host = self.host.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(host.as_ref())
    }

    const fn frame(got: Option<i32>, timestamp: u64, gone: MpRecv) -> MpRecv {
        match got {
            None => gone,
            Some(n) if n <= 0 => MpRecv::Empty,
            Some(n) => MpRecv::Frame { len: n as usize, timestamp },
        }
    }
}

impl MpLink for HostLink {
    fn begin(&mut self) {
        self.with(|h| h.mp_begin());
    }
    fn end(&mut self) {
        self.with(|h| h.mp_end());
    }
    fn clock(&mut self, now: u64) {
        self.now = now;
        self.with(|h| h.mp_clock(now));
    }
    fn send_packet(&mut self, frame: &[u8], timestamp: u64) {
        self.with(|h| h.mp_send_packet(frame, timestamp));
    }
    fn send_cmd(&mut self, frame: &[u8], timestamp: u64) {
        self.with(|h| h.mp_send_cmd(frame, timestamp));
    }
    fn send_reply(&mut self, frame: &[u8], timestamp: u64, aid: u16) {
        self.with(|h| h.mp_send_reply(frame, timestamp, aid));
    }
    fn send_ack(&mut self, frame: &[u8], timestamp: u64) {
        self.with(|h| h.mp_send_ack(frame, timestamp));
    }
    fn recv_packet(&mut self, buf: &mut [u8]) -> MpRecv {
        let (now, mut ts) = (self.now, 0);
        let got = self.with(|h| h.mp_recv_packet(buf, now, &mut ts));
        Self::frame(got, ts, MpRecv::Empty)
    }
    fn recv_host_packet(&mut self, buf: &mut [u8]) -> MpRecv {
        let (now, mut ts) = (self.now, 0);
        let got = self.with(|h| h.mp_recv_host_packet(buf, now, &mut ts));
        Self::frame(got, ts, MpRecv::HostGone)
    }
    fn recv_replies(&mut self, buf: &mut [u8], timestamp: u64, aidmask: u16) -> u16 {
        let now = self.now;
        self.with(|h| h.mp_recv_replies(buf, now, timestamp, aidmask))
    }
}

/// One emulated DS (melonds `Nds`) on the Lunaris core.
pub struct Nds {
    emu: Box<Emulator>,
    host: Arc<Mutex<Box<dyn Host>>>,
    /// The ROM between `new` and the first `boot` (the cart owns it after).
    rom: Vec<u8>,
    first_save: Option<Vec<u8>>,
    booted: bool,
    running: bool,
    frames: u32,
    top: Vec<u32>,
    bottom: Vec<u32>,
    have_frame: bool,
    mic_static: bool,
    /// `set_render`: whether the framebuffers are updated at all.
    render: bool,
    /// `set_displayed_screens`: bit 0 top, bit 1 bottom.
    screens: u8,
    samples: Vec<i16>,
    resampler: Resampler,
}

impl Nds {
    /// A console with `rom` in the slot and `save` in its save chip.
    /// `instance_id` makes the Wi-Fi MAC unique for a linked pair.
    pub fn new(
        rom: &[u8],
        save: Option<&[u8]>,
        instance_id: u32,
        host: Box<dyn Host>,
    ) -> Result<Self, Error> {
        if rom.len() < 0x200 {
            return Err(Error::BadRom);
        }
        let mut emu = Box::new(Emulator::new());
        emu.config.mac_address = Some([0x00, 0x09, 0xBF, 0x11, 0x22, 0x33 + instance_id as u8]);
        let host = Arc::new(Mutex::new(host));
        emu.wifi.set_link(Some(Box::new(HostLink { host: host.clone(), now: 0 })));
        Ok(Self {
            emu,
            host,
            rom: rom.to_vec(),
            first_save: save.map(<[u8]>::to_vec),
            booted: false,
            running: false,
            frames: 0,
            top: vec![0; SCREEN_WIDTH * SCREEN_HEIGHT],
            bottom: vec![0; SCREEN_WIDTH * SCREEN_HEIGHT],
            have_frame: false,
            mic_static: false,
            render: true,
            screens: 3,
            samples: Vec::new(),
            resampler: Resampler::default(),
        })
    }

    fn host(&self) -> std::sync::MutexGuard<'_, Box<dyn Host>> {
        self.host.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The direct core, for what the melonds API does not cover.
    pub fn core(&mut self) -> &mut Emulator {
        &mut self.emu
    }

    /// Reset and direct-boot into the cart (keeping the save memory).
    pub fn boot(&mut self) {
        let (rom, save) = if self.booted {
            (std::mem::take(&mut self.emu.cart.rom), Some(self.emu.cart.backup.data().to_vec()))
        } else {
            (std::mem::take(&mut self.rom), self.first_save.take())
        };
        let name =
            String::from_utf8_lossy(rom.get(..12).unwrap_or(&[])).trim_end_matches('\0').to_owned();
        self.emu.load_rom_image(rom, &name, save.as_deref());
        self.emu.cart.backup.take_dirty();
        self.emu.wifi.set_frame_timebase(Some(u64::from(self.frames)));
        self.booted = true;
        self.running = true;
        self.have_frame = false;
        self.resampler = Resampler::default();
    }

    /// Runs one video frame; returns the number of scanlines (263).
    pub fn run_frame(&mut self) -> u32 {
        if !self.running {
            return 0;
        }
        if self.mic_static {
            self.emu.mic_blow();
        }
        self.emu.run();
        self.frames = self.frames.wrapping_add(1);
        // The core always renders (display capture depends on it); what is
        // not wanted is just not copied out.
        if self.render {
            if self.screens & 1 != 0 {
                self.emu.get_upper_frame(&mut self.top);
            }
            if self.screens & 2 != 0 {
                self.emu.get_lower_frame(&mut self.bottom);
            }
            self.have_frame = true;
        }

        self.samples.clear();
        self.emu.take_audio_samples(&mut self.samples);
        self.resampler.push(&self.samples);

        if self.emu.cart.backup.take_dirty() {
            let data = self.emu.cart.backup.data().to_vec();
            self.host().write_save(&data, 0, data.len() as u32);
        }
        if self.emu.power_off_requested() {
            self.running = false;
            self.host().signal_stop(1); // melonDS StopReason::PowerOff
        }
        263
    }

    pub const fn is_running(&mut self) -> bool {
        self.running
    }

    pub const fn frame_count(&mut self) -> u32 {
        self.frames
    }

    /// Moves the frame counter, which is also the Wi-Fi clock's timebase.
    pub fn set_frame_count(&mut self, frames: u32) {
        self.frames = frames;
        self.emu.wifi.set_frame_timebase(Some(u64::from(frames)));
    }

    pub fn set_lid_closed(&mut self, closed: bool) {
        self.emu.set_lid_closed(closed);
    }

    pub fn lid_closed(&mut self) -> bool {
        self.emu.lid_closed()
    }

    pub fn set_battery_okay(&mut self, okay: bool) {
        self.emu.spi.set_battery_low(!okay);
    }

    pub fn battery_okay(&mut self) -> bool {
        !self.emu.spi.battery_low()
    }

    /// Sets the RTC to this date/time; it keeps running from there.
    pub fn set_rtc(
        &mut self,
        year: i32,
        month: i32,
        day: i32,
        hour: i32,
        minute: i32,
        second: i32,
    ) {
        let u = |v: i32| v.max(0) as u32;
        self.emu.rtc.set_running_time((year, u(month), u(day), u(hour), u(minute), u(second)));
    }

    /// Held keys (active-high, see [`keys`]).
    pub fn set_keys(&mut self, k: u32) {
        let on = |bit: u32| k & bit != 0;
        let key = &mut self.emu.key_input;
        key.button_a = on(keys::A);
        key.button_b = on(keys::B);
        key.select = on(keys::SELECT);
        key.start = on(keys::START);
        key.right = on(keys::RIGHT);
        key.left = on(keys::LEFT);
        key.up = on(keys::UP);
        key.down = on(keys::DOWN);
        key.button_r = on(keys::R);
        key.button_l = on(keys::L);
        self.emu.ext_key_in.button_x = on(keys::X);
        self.emu.ext_key_in.button_y = on(keys::Y);
    }

    pub fn touch(&mut self, x: u16, y: u16) {
        self.emu.touchscreen_press(i32::from(x), i32::from(y));
    }

    pub fn release_screen(&mut self) {
        self.emu.touchscreen_press(0, 0xFFF);
    }

    /// Mic input as white noise while on (set per frame).
    pub const fn set_mic_static(&mut self, on: bool) {
        self.mic_static = on;
    }

    /// Whether the framebuffers follow the console (off: they hold the
    /// last picture; emulation is unchanged).
    pub const fn set_render(&mut self, enabled: bool) {
        self.render = enabled;
    }

    /// Which framebuffers anyone looks at: bit 0 top, bit 1 bottom.
    pub const fn set_displayed_screens(&mut self, screens: u8) {
        self.screens = screens;
    }

    /// Top and bottom screens, 0xAARRGGBB, 256x192 each.
    pub fn framebuffers(&mut self) -> Option<(&[u32], &[u32])> {
        self.have_frame.then_some((&self.top[..], &self.bottom[..]))
    }

    /// Takes up to `out.len() / 2` stereo pairs at 48 kHz; returns pairs.
    pub fn read_audio(&mut self, out: &mut [i16]) -> usize {
        self.resampler.pop(out)
    }

    pub fn audio_queued(&mut self) -> usize {
        self.resampler.queued()
    }

    /// The 4 MiB main RAM.
    pub fn main_ram(&mut self) -> &mut [u8] {
        &mut self.emu.main_ram
    }

    pub fn read32(&mut self, addr: u32) -> u32 {
        self.emu.read_word(addr, ds_emu::CpuType::Arm9)
    }
    pub fn read16(&mut self, addr: u32) -> u16 {
        self.emu.read_halfword(addr, ds_emu::CpuType::Arm9)
    }
    pub fn read8(&mut self, addr: u32) -> u8 {
        self.emu.read_byte(addr, ds_emu::CpuType::Arm9)
    }
    pub fn write32(&mut self, addr: u32, val: u32) {
        self.emu.write_word(addr, val, ds_emu::CpuType::Arm9);
    }
    pub fn write16(&mut self, addr: u32, val: u16) {
        self.emu.write_halfword(addr, val, ds_emu::CpuType::Arm9);
    }
    pub fn write8(&mut self, addr: u32, val: u8) {
        self.emu.write_byte(addr, val, ds_emu::CpuType::Arm9);
    }

    pub fn arm7_read32(&mut self, addr: u32) -> u32 {
        self.emu.read_word(addr, ds_emu::CpuType::Arm7)
    }
    pub fn arm7_read16(&mut self, addr: u32) -> u16 {
        self.emu.read_halfword(addr, ds_emu::CpuType::Arm7)
    }
    pub fn arm7_read8(&mut self, addr: u32) -> u8 {
        self.emu.read_byte(addr, ds_emu::CpuType::Arm7)
    }
    pub fn arm7_write32(&mut self, addr: u32, val: u32) {
        self.emu.write_word(addr, val, ds_emu::CpuType::Arm7);
    }
    pub fn arm7_write16(&mut self, addr: u32, val: u16) {
        self.emu.write_halfword(addr, val, ds_emu::CpuType::Arm7);
    }
    pub fn arm7_write8(&mut self, addr: u32, val: u8) {
        self.emu.write_byte(addr, val, ds_emu::CpuType::Arm7);
    }

    pub fn pc(&mut self) -> u32 {
        self.emu.arm9.get_pc()
    }

    pub fn arm7_pc(&mut self) -> u32 {
        self.emu.arm7.get_pc()
    }

    /// A savestate in the core's own format.
    pub fn save_state(&mut self, buf: &mut Vec<u8>) -> Result<(), Error> {
        *buf = self.emu.save_state().map_err(|_| Error::Savestate)?;
        Ok(())
    }

    pub fn load_state(&mut self, buf: &[u8]) -> Result<(), Error> {
        self.emu.load_state(buf).map_err(|_| Error::Savestate)?;
        self.emu.cart.backup.take_dirty();
        self.running = true;
        Ok(())
    }

    /// The cart's save memory.
    pub fn save_memory(&mut self) -> Vec<u8> {
        if self.booted {
            self.emu.cart.backup.data().to_vec()
        } else {
            self.first_save.clone().unwrap_or_default()
        }
    }

    /// Installs the Action Replay codes (only enabled ones run).
    pub fn set_cheats(&mut self, cheats: &[Cheat]) {
        self.emu.set_cheats(cheats.iter().filter(|c| c.enabled).map(|c| c.code.clone()).collect());
    }

    /// Only the software renderer exists; `threaded` sets the 3D worker
    /// threads.
    pub fn set_render_settings(&mut self, settings: RenderSettings) -> Renderer {
        ds_gpu::set_render_threads(if settings.threaded {
            ds_gpu::DEFAULT_RENDER_THREADS
        } else {
            1
        });
        Renderer::Software
    }

    pub const fn renderer(&mut self) -> Renderer {
        Renderer::Software
    }

    pub const fn gl_output(&mut self) -> Option<GlOutput> {
        None
    }

    pub const fn gl_shader_compile_step(&mut self) -> Option<(u32, u32)> {
        None
    }

    pub const fn gl_read_output(&mut self, _screen: u8, _out: &mut [u32]) -> usize {
        0
    }
}
