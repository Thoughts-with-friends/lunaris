//! One running DS for the front ends: the core (`ds_emu`) plus what a GUI
//! needs around it.
//!
//! ```text
//!  front end (egui / freya)
//!    |  press_key / press_screen / emulate_frame / get_screens / ...
//!    v
//!  Console
//!    +-- ds_emu::Emulator    the DS (CPUs, memory, GPU, SPU, Wi-Fi, cart)
//!    +-- AudioOutput         SPU samples -> cpal (or kept for take_audio)
//!    +-- MP link             other consoles: `ds_net::NetLink` (melonDS
//!    |                       LocalMP / LAN), set by the front end
//!    +-- screens             both screens of the last frame (0xAARRGGBB)
//!    `-- cheat map           the editor's codes; the enabled ones run
//! ```
//!
//! The core crates stay GUI-free: savestates, debug views and Wi-Fi
//! counters live in them, multiplayer in `ds_net`; this type only forwards.
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use ds_emu::Emulator;
pub use ds_emu::{MpLink, RomFingerprint, StateError};
pub use ds_gpu::debug::{DebugImage, Engine, GraphicsType};
/// The association trace switch and its lines (`set`, `lines`, `clear`).
pub use ds_peripherals::wifi::trace as assoc_trace;
pub use ds_peripherals::wifi::{MpDiag, RxDrops};

use crate::{audio::AudioOutput, cheat_map::CheatMap};

/// Width and height of one screen.
pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 192;

/// Interleaved samples [`Console::audio_tap`] keeps (about 0.5 s of stereo).
pub const AUDIO_TAP_LEN: usize = 32768;

/// A DS button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    A,
    B,
    Select,
    Start,
    Right,
    Left,
    Up,
    Down,
    R,
    L,
    X,
    Y,
}

/// What to boot.
#[derive(Debug, Clone, Copy)]
pub struct Boot<'a> {
    pub rom: &'a Path,
    /// `<save_dir>/<rom name>.sav` is the save file.
    pub save_dir: &'a Path,
    /// BIOS / firmware dumps; `None` (or unreadable): the built-in ones.
    pub bios7: Option<&'a Path>,
    pub bios9: Option<&'a Path>,
    pub firmware: Option<&'a Path>,
    /// Wi-Fi MAC written into the firmware (`None`: the firmware's).
    pub mac: Option<[u8; 6]>,
    /// Plays its own audio as instance `n` (one output stream per
    /// instance); `None`: the caller collects it with
    /// [`Console::take_audio`].
    pub audio_instance: Option<u8>,
    /// 0-100.
    pub volume: f32,
}

/// The output stream of instance `n`, made once and shared by every console
/// of that instance (a stream is kept for the life of the process).
fn audio_output(instance: u8) -> AudioOutput {
    static OUTPUTS: OnceLock<Mutex<HashMap<u8, AudioOutput>>> = OnceLock::new();
    let outputs = OUTPUTS.get_or_init(Mutex::default);
    let mut outputs = outputs.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    outputs.entry(instance).or_insert_with(|| AudioOutput::new(1.0)).clone()
}

pub struct Console {
    emu: Box<Emulator>,
    /// No ROM: never stepped (the start screen's stand-in).
    has_rom: bool,
    audio: Option<AudioOutput>,
    /// Interleaved stereo samples of the frames since the last
    /// [`Self::take_audio`] (only without an own audio output).
    samples: Vec<i16>,
    /// The last [`AUDIO_TAP_LEN`] interleaved stereo samples, whoever
    /// plays them (for scopes and spectrograms).
    audio_tap: VecDeque<i16>,
    audio_sync: bool,
    screens: [Vec<u32>; 2],
    cheat_map: CheatMap,
    cheats_enabled: bool,
}

impl Console {
    /// Boots `boot.rom`.
    ///
    /// # Errors
    /// If the ROM or its save file cannot be read.
    pub fn new(boot: &Boot<'_>) -> Result<Self, String> {
        let mut emu = Box::new(Emulator::new());
        emu.config.save_dir = Some(boot.save_dir.to_owned());
        emu.config.mac_address = boot.mac;
        let read = |path: Option<&Path>| path.and_then(|p| std::fs::read(p).ok());
        if let Some(bios) = read(boot.bios7) {
            emu.load_bios7(&bios);
        }
        if let Some(bios) = read(boot.bios9) {
            emu.load_bios9(&bios);
        }
        if let Some(firmware) = read(boot.firmware) {
            emu.load_firmware_image(&firmware);
        }
        let mut console = Self::with_emulator(emu, boot.rom, boot.audio_instance)?;
        console.set_audio_volume(boot.volume);
        Ok(console)
    }

    /// Boots `rom` on an emulator the caller configured (settings, BIOS,
    /// firmware, save directory); audio as [`Boot::audio_instance`].
    ///
    /// # Errors
    /// If the ROM or its save file cannot be read.
    pub fn with_emulator(
        mut emu: Box<Emulator>,
        rom: &Path,
        audio_instance: Option<u8>,
    ) -> Result<Self, String> {
        emu.load_rom(rom).map_err(|e| e.to_string())?;
        Ok(Self::with(emu, true, audio_instance.map(audio_output)))
    }

    /// A console without a game, standing in until one is chosen. Emulating
    /// it does nothing.
    pub fn placeholder() -> Self {
        Self::with(Box::new(Emulator::new()), false, None)
    }

    fn with(emu: Box<Emulator>, has_rom: bool, audio: Option<AudioOutput>) -> Self {
        Self {
            emu,
            has_rom,
            audio,
            samples: Vec::new(),
            audio_tap: VecDeque::with_capacity(AUDIO_TAP_LEN),
            audio_sync: false,
            screens: [vec![0xFF00_0000; WIDTH * HEIGHT], vec![0xFF00_0000; WIDTH * HEIGHT]],
            cheat_map: CheatMap::new(),
            cheats_enabled: false,
        }
    }

    /// Whether a game is loaded (see [`Self::placeholder`]).
    pub const fn has_rom(&self) -> bool {
        self.has_rom
    }

    pub fn emulator(&self) -> &Emulator {
        &self.emu
    }

    pub fn emulator_mut(&mut self) -> &mut Emulator {
        &mut self.emu
    }

    /// Runs one video frame and plays (or keeps) its audio.
    pub fn emulate_frame(&mut self) {
        if !self.has_rom {
            return;
        }
        self.emu.run();
        if let Some(audio) = &self.audio {
            self.samples.clear();
            self.emu.take_audio_samples(&mut self.samples);
            Self::tap(&mut self.audio_tap, &self.samples);
            audio.push(&self.samples);
            if self.audio_sync {
                audio.wait_for_room(Duration::from_millis(50));
            }
        } else {
            let start = self.samples.len();
            self.emu.take_audio_samples(&mut self.samples);
            Self::tap(&mut self.audio_tap, &self.samples[start..]);
            // Nobody collects them: keep about a second.
            let keep = 2 * ds_spu::OUTPUT_RATE_HZ as usize;
            if self.samples.len() > keep {
                self.samples.drain(..self.samples.len() - keep);
            }
        }
        let [upper, lower] = &mut self.screens;
        self.emu.get_upper_frame(upper);
        self.emu.get_lower_frame(lower);
    }

    fn tap(tap: &mut VecDeque<i16>, samples: &[i16]) {
        let samples = &samples[samples.len().saturating_sub(AUDIO_TAP_LEN)..];
        let overflow = (tap.len() + samples.len()).saturating_sub(AUDIO_TAP_LEN);
        tap.drain(..overflow);
        tap.extend(samples);
    }

    /// The most recent output, interleaved stereo at
    /// `ds_spu::OUTPUT_RATE_HZ`, oldest first; does not consume anything.
    pub const fn audio_tap(&self) -> &VecDeque<i16> {
        &self.audio_tap
    }

    /// The audio since the last call (interleaved stereo `i16` at
    /// `ds_spu::OUTPUT_RATE_HZ`), for a console without its own output.
    pub fn take_audio(&mut self, out: &mut Vec<i16>) {
        out.append(&mut self.samples);
    }

    /// Upper and lower screen of the last frame, `0xAARRGGBB` per pixel.
    pub fn get_screens(&self) -> [&[u32]; 2] {
        [&self.screens[0], &self.screens[1]]
    }

    fn set_key(&mut self, key: Key, down: bool) {
        let k = &mut self.emu.key_input;
        let e = &mut self.emu.ext_key_in;
        match key {
            Key::A => k.button_a = down,
            Key::B => k.button_b = down,
            Key::Select => k.select = down,
            Key::Start => k.start = down,
            Key::Right => k.right = down,
            Key::Left => k.left = down,
            Key::Up => k.up = down,
            Key::Down => k.down = down,
            Key::R => k.button_r = down,
            Key::L => k.button_l = down,
            Key::X => e.button_x = down,
            Key::Y => e.button_y = down,
        }
    }

    pub fn press_key(&mut self, key: Key) {
        self.set_key(key, true);
    }

    pub fn release_key(&mut self, key: Key) {
        self.set_key(key, false);
    }

    /// Touches the lower screen at DS pixel (`x`, `y`).
    pub fn press_screen(&mut self, x: usize, y: usize) {
        self.emu.touchscreen_press(x.min(WIDTH - 1) as i32, y.min(HEIGHT - 1) as i32);
    }

    pub fn release_screen(&mut self) {
        // Y = FFFh: pen up (DS Touch Screen Controller).
        self.emu.touchscreen_press(0, 0xFFF);
    }

    /// 0-100 %.
    pub fn set_audio_volume(&mut self, percent: f32) {
        if let Some(audio) = &self.audio {
            audio.set_volume(percent / 100.0);
        }
    }

    /// Hold [`Self::emulate_frame`] back while the audio queue is full, so
    /// emulation runs at the speed the sound card plays (native speed).
    pub const fn set_audio_sync(&mut self, sync: bool) {
        self.audio_sync = sync;
    }

    pub const fn get_cheat_map(&self) -> &CheatMap {
        &self.cheat_map
    }

    pub fn set_cheat_map(&mut self, cheat_map: CheatMap) {
        self.cheat_map = cheat_map;
        self.apply_cheats();
    }

    pub fn set_enable_cheats(&mut self, enable: bool) {
        self.cheats_enabled = enable;
        self.apply_cheats();
    }

    fn apply_cheats(&mut self) {
        let codes = if self.cheats_enabled {
            self.cheat_map.iter().filter(|c| c.enabled).map(|c| c.code.clone()).collect()
        } else {
            Vec::new()
        };
        self.emu.set_cheats(codes);
    }

    /// The core's state (see `ds_emu::Emulator::save_state`).
    ///
    /// # Errors
    /// If the state cannot be encoded.
    pub fn save_state(&self) -> Result<Vec<u8>, StateError> {
        self.emu.save_state()
    }

    /// # Errors
    /// If `state` is not a state of this core.
    pub fn load_state(&mut self, state: &[u8]) -> Result<(), StateError> {
        self.emu.load_state(state)?;
        let [upper, lower] = &mut self.screens;
        self.emu.get_upper_frame(upper);
        self.emu.get_lower_frame(lower);
        Ok(())
    }

    pub fn rom_fingerprint(&self) -> RomFingerprint {
        self.emu.rom_fingerprint()
    }

    /// Replaces the save memory with a `.sav` / `.dsv` file's contents.
    pub fn import_save(&mut self, file: &[u8]) {
        self.emu.import_save(file);
    }

    /// The save memory as a raw `.sav` image.
    pub fn export_save(&self) -> Vec<u8> {
        self.emu.export_save()
    }

    /// Writes the save memory to its `.sav` if it changed (errors are
    /// logged).
    pub fn flush_save(&mut self) {
        if let Err(e) = self.emu.flush_save() {
            log::error!(target: "ds_cartridge", "{e}");
        }
    }

    /// Connects the Wi-Fi to other consoles (a `ds_net::NetLink`; `None`:
    /// nobody in range).
    pub fn set_mp_link(&mut self, link: Option<Box<dyn MpLink>>) {
        self.emu.set_mp_link(link);
    }

    /// Whether an MP link is installed.
    pub const fn has_mp_link(&self) -> bool {
        self.emu.wifi.has_link()
    }

    pub fn wifi_diag_snapshot(&self) -> MpDiag {
        self.emu.wifi.diag()
    }

    /// The Wi-Fi clock (µs), the timebase MP frames are stamped with.
    pub const fn wifi_clock(&self) -> u64 {
        self.emu.wifi.clock()
    }

    pub fn render_palettes(
        &self,
        extended: bool,
        slot: usize,
        palette: usize,
        engine: Engine,
        kind: GraphicsType,
    ) -> DebugImage {
        self.emu.gpu.render_palettes(extended, slot, palette, engine, kind)
    }

    pub fn render_map(&self, engine: Engine, bg: usize) -> DebugImage {
        self.emu.gpu.render_map(engine, bg)
    }

    #[expect(clippy::too_many_arguments, reason = "one parameter per debugger control")]
    pub fn render_tiles(
        &self,
        engine: Engine,
        kind: GraphicsType,
        extended: bool,
        bitmap: bool,
        bpp8: bool,
        slot: usize,
        palette: usize,
        offset: usize,
    ) -> DebugImage {
        self.emu.gpu.render_tiles(engine, kind, extended, bitmap, bpp8, slot, palette, offset)
    }

    pub fn render_bank(&self, bank: usize, ignore_alpha: bool) -> DebugImage {
        self.emu.gpu.render_bank(bank, ignore_alpha)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_does_not_run() {
        let mut console = Console::placeholder();
        console.emulate_frame();
        assert!(!console.has_rom());
        assert_eq!(console.get_screens()[0].len(), WIDTH * HEIGHT);
    }

    #[test]
    fn keys_reach_the_core() {
        let mut console = Console::placeholder();
        console.press_key(Key::X);
        console.press_key(Key::Start);
        assert!(console.emulator().ext_key_in.button_x);
        assert!(console.emulator().key_input.start);
        console.release_key(Key::X);
        assert!(!console.emulator().ext_key_in.button_x);
    }

    #[test]
    fn only_enabled_cheats_run() {
        use crate::cheat_map::ArCode;
        let mut console = Console::placeholder();
        console.set_cheat_map(vec![
            ArCode { code: vec![1, 2], enabled: true },
            ArCode { code: vec![3, 4], enabled: false },
        ]);
        assert!(console.emulator().cheats.is_empty(), "cheats are off");
        console.set_enable_cheats(true);
        assert_eq!(console.emulator().cheats, vec![vec![1, 2]]);
    }
}
