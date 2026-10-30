//! The frontend state and emulator thread of this GUI: `ds_common`'s
//! `Frontend` / `EmuThread`, running the core this build was made with
//! ([`BACKEND`]) instead of only cogids-rs's core.
//!
//! ```text
//!  GUI menu click --Action--> Frontend::apply()   (start / stop / pause /
//!                                                  view / speed / cheats)
//!  Frontend::start(rom) --backend::loader--> EmuThread
//!     emulator thread: Loader() -> Box<dyn Core>   (load result sent back)
//!       loop: keys + touch -> run_frame -> audio -> frame
//!         1x:  Core::frame -> Shared::frame
//!         2x+: Core::hd_job --try--> HD worker -> Shared::frame
//! ```
//!
//! The GUI config is `freya_config.json` in the instance folder: the
//! lunaris egui GUI keeps a `config.json` of another shape in the same
//! `instances/instanceN` folder.
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use ds_common::{
    AudioOutput, Instance, Settings, cheats,
    frontend::{GuiConfig, file_name, lock},
    menu::{Action, remember},
    speed,
    view::View,
};

use crate::backend::{self, BACKEND, Boot, Core, DebugView, HdJob, Image, Loader, panic_message};

/// Frames a touch is kept down after release, so quick taps register.
const TOUCH_MIN_FRAMES: u32 = 4;
/// Frames between two pictures of an open Debug view (~10 a second).
const DEBUG_EVERY: u64 = 6;
/// Frame times kept for Debug > Performance stats (20 s).
pub const FRAME_TIMES: usize = 20 * 60;

/// How long the UI waits for the emulator thread to carry out a command.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// Work the emulator thread does between frames, for the UI.
#[cfg_attr(not(feature = "lunaris"), allow(dead_code, reason = "LAN play is Lunaris-only"))]
pub enum Command {
    SaveState(PathBuf),
    LoadState(PathBuf),
    /// Write the save file now (Export savefile).
    FlushSave,
    /// Install or remove the LAN link (`crate::lan`).
    SetLink(Option<Box<dyn std::any::Any + Send>>),
    /// The emulated Wi-Fi handshake stages, one per line.
    WifiStatus,
}

/// A command and where its result (text for [`Command::WifiStatus`]) goes.
type Pending = (Command, SyncSender<Result<String, String>>);

#[derive(Default)]
struct TouchState {
    pos: Option<(i32, i32)>,
    /// The frontend released the stylus.
    released: bool,
    /// Frames the current touch has been held.
    held: u32,
}

/// State shared between the UI (commands) and the emulator thread.
#[derive(Default)]
pub struct Shared {
    running: AtomicBool,
    keys: AtomicU32,
    touch: Mutex<TouchState>,
    /// Upper then lower screen, RGBA, `frame_width` pixels wide.
    frame: Mutex<Vec<u8>>,
    frame_width: AtomicU32,
    frame_seq: AtomicU64,
    /// The frontend fetched the last frame (the next one is worth making).
    frame_taken: AtomicBool,
    fps: AtomicU32,
    /// System > Pause.
    paused: AtomicBool,
    /// System > Frame step: run one frame while paused.
    step: AtomicBool,
    /// Config > Limit framerate turned off.
    unlimited: AtomicBool,
    /// Emulation speed in percent (0 = not set yet, 1x).
    speed: AtomicU32,
    /// Internal resolution factor (bits 0-7, 0 = 1x) and xBRZ (bit 8).
    render: AtomicU32,
    /// Cheat codes to hand to the core at the next frame.
    cheats: Mutex<Option<Vec<Vec<u32>>>>,
    /// Commands for the emulator thread, carried out even while paused.
    commands: Mutex<Vec<Pending>>,
    /// Why the core stopped on its own (a panic), until the UI shows it.
    crash: Mutex<Option<String>>,
    /// The open Debug view, drawn by the emulator thread.
    debug_view: Mutex<Option<DebugView>>,
    /// Its latest picture and a sequence number.
    debug_image: Mutex<Option<(u64, Image)>>,
    /// Seconds between the last frames (Performance stats).
    frame_times: Mutex<VecDeque<f32>>,
}

impl Shared {
    pub fn set_keys(&self, keys: u32) {
        self.keys.store(keys, Ordering::Relaxed);
    }

    /// `Some((x, y))` presses (or moves) the stylus, `None` lifts it.
    pub fn set_touch(&self, pos: Option<(i32, i32)>) {
        let mut t = lock(&self.touch);
        match pos {
            Some((x, y)) => {
                if t.pos.is_none() {
                    t.held = 0;
                }
                t.pos = Some((x.clamp(0, 255), y.clamp(0, 191)));
                t.released = false;
            }
            None => t.released = true,
        }
    }

    /// The latest frame if it is newer than `last_seq`: `(seq, width,
    /// rgba)` (both screens, upper first, `width` pixels wide each).
    pub fn frame_since(&self, last_seq: u64) -> Option<(u64, u32, Vec<u8>)> {
        let seq = self.frame_seq.load(Ordering::Acquire);
        if seq == last_seq || seq == 0 {
            return None;
        }
        let rgba = lock(&self.frame).clone();
        if rgba.is_empty() {
            return None;
        }
        let width = self.frame_width.load(Ordering::Acquire);
        self.frame_taken.store(true, Ordering::Release);
        Some((seq, width, rgba))
    }

    /// Emulation speed in percent (`speed::SPEEDS`).
    pub fn set_speed(&self, percent: u16) {
        self.speed.store(u32::from(speed::valid(percent)), Ordering::Relaxed);
    }

    /// Internal resolution and the xBRZ filter of the frames made from now.
    pub fn set_render(&self, scale: u8, xbrz: bool) {
        let bits = u32::from(scale) | (u32::from(xbrz) << 8);
        self.render.store(bits, Ordering::Relaxed);
    }

    /// Replaces the running game's cheat codes (from the next frame).
    pub fn set_cheats(&self, codes: Vec<Vec<u32>>) {
        *lock(&self.cheats) = Some(codes);
    }

    /// Emulated frames per second over the last second.
    pub fn fps(&self) -> f32 {
        f32::from_bits(self.fps.load(Ordering::Relaxed))
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// What the emulator thread draws for the Debug panel (`None`: nothing).
    pub fn set_debug_view(&self, view: Option<DebugView>) {
        *lock(&self.debug_view) = view;
        if view.is_none() {
            *lock(&self.debug_image) = None;
        }
    }

    /// The Debug view's picture, if newer than `last_seq`.
    pub fn debug_image_since(&self, last_seq: u64) -> Option<(u64, Image)> {
        lock(&self.debug_image).as_ref().filter(|(seq, _)| *seq != last_seq).cloned()
    }

    /// The frame times of the last [`FRAME_TIMES`] frames, oldest first.
    pub fn frame_times(&self) -> Vec<f32> {
        lock(&self.frame_times).iter().copied().collect()
    }

    fn push_frame_time(&self, seconds: f32) {
        let mut times = lock(&self.frame_times);
        if times.len() == FRAME_TIMES {
            times.pop_front();
        }
        times.push_back(seconds);
    }

    /// Draws the open Debug view into the mailbox.
    fn draw_debug(&self, core: &mut dyn Core) {
        let view = *lock(&self.debug_view);
        if let Some(image) = view.and_then(|v| core.debug_image(v)) {
            let mut slot = lock(&self.debug_image);
            let seq = slot.as_ref().map_or(1, |(s, _)| s + 1);
            *slot = Some((seq, image));
        }
    }

    /// The message of a core crash, once.
    pub fn take_crash(&self) -> Option<String> {
        lock(&self.crash).take()
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Pauses and runs exactly one frame.
    pub fn frame_step(&self) {
        self.paused.store(true, Ordering::Relaxed);
        self.step.store(true, Ordering::Relaxed);
    }

    pub fn set_limit_framerate(&self, limit: bool) {
        self.unlimited.store(!limit, Ordering::Relaxed);
    }

    /// Frames produced so far (the frame sequence number).
    #[cfg(test)]
    pub fn frames(&self) -> u64 {
        self.frame_seq.load(Ordering::Acquire)
    }

    /// The stylus for this frame (a quick tap is held a few frames).
    fn take_touch(&self) -> Option<(i32, i32)> {
        let mut t = lock(&self.touch);
        let pos = t.pos?;
        t.held += 1;
        if t.released && t.held >= TOUCH_MIN_FRAMES {
            t.pos = None;
            t.released = false;
        }
        drop(t);
        Some(pos)
    }
}

/// A running emulator thread.
pub struct EmuThread {
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
}

impl EmuThread {
    /// Builds the core with `loader` on a new thread and runs it.
    ///
    /// # Errors
    /// If the core cannot be built (the thread has ended by then).
    pub fn start(loader: Loader, shared: Arc<Shared>, audio: AudioOutput) -> Result<Self, String> {
        shared.running.store(true, Ordering::Relaxed);
        let (loaded, load_result) = std::sync::mpsc::sync_channel(1);
        let thread_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("emulator".to_owned())
            .spawn(move || {
                let core = std::panic::catch_unwind(std::panic::AssertUnwindSafe(loader))
                    .unwrap_or_else(|panic| Err(panic_message(&*panic)));
                match core {
                    Ok(core) => {
                        let _ = loaded.send(Ok(()));
                        run(core, &thread_shared, &audio);
                    }
                    Err(e) => {
                        thread_shared.running.store(false, Ordering::Relaxed);
                        let _ = loaded.send(Err(e));
                    }
                }
            })
            .map_err(|e| {
                shared.running.store(false, Ordering::Relaxed);
                format!("Failed to start emulator thread: {e}")
            })?;
        let thread = Self { shared, handle: Some(handle) };
        match load_result.recv() {
            Ok(Ok(())) => Ok(thread),
            Ok(Err(e)) => {
                thread.stop();
                Err(e)
            }
            Err(_) => {
                thread.stop();
                Err("The emulator thread stopped while loading".to_owned())
            }
        }
    }

    /// Stops the thread; the core writes its save first.
    pub fn stop(mut self) {
        self.shared.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            log::error!("Emulator thread panicked");
        }
    }
}

/// Makes `rgba` (both screens, `width` pixels wide) the latest frame.
fn store_frame(shared: &Shared, rgba: &mut Vec<u8>, width: usize) {
    std::mem::swap(&mut *lock(&shared.frame), rgba);
    shared.frame_width.store(width as u32, Ordering::Release);
}

/// Publishes the frame the core just finished: at 1x (or for a core that
/// cannot upscale) right here, above it through the HD worker. `wait`
/// (single steps) waits for a busy worker instead of dropping the frame.
fn publish_frame(
    core: &mut dyn Core,
    shared: &Shared,
    hd: &SyncSender<Box<dyn HdJob>>,
    wait: bool,
    rgba: &mut Vec<u8>,
) {
    let render = shared.render.load(Ordering::Relaxed);
    let scale = (render & 0xFF).clamp(1, 4) as usize;
    let xbrz = render & 0x100 != 0 && scale >= 2;
    if scale > 1
        && let Some(job) = core.hd_job(scale, xbrz)
    {
        if wait {
            let _ = hd.send(job);
        } else if let Err(TrySendError::Disconnected(_)) = hd.try_send(job) {
            log::error!("HD worker stopped");
        }
        return;
    }
    rgba.clear();
    core.frame(rgba);
    store_frame(shared, rgba, 256);
}

/// The HD worker: upscales the frames it is handed until the emulator
/// thread hangs up.
fn hd_worker(shared: &Shared, jobs: &Receiver<Box<dyn HdJob>>) {
    let mut rgba = Vec::new();
    for job in jobs {
        rgba.clear();
        let width = job.render(&mut rgba);
        store_frame(shared, &mut rgba, width);
    }
}

fn run(mut core: Box<dyn Core>, shared: &Shared, audio: &AudioOutput) {
    std::thread::scope(|s| {
        // Rendezvous: a job is only taken while the worker is idle.
        let (hd, jobs) = std::sync::mpsc::sync_channel(0);
        let worker = std::thread::Builder::new()
            .name("hd-render".to_owned())
            .spawn_scoped(s, move || hd_worker(shared, &jobs));
        if let Err(e) = worker {
            log::error!("Failed to start the HD worker: {e}");
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            emulate(&mut *core, shared, audio, &hd);
        }));
        drop(hd);
        audio.clear();
        if let Err(panic) = result {
            let message = panic_message(&*panic);
            log::error!("The core crashed: {message}");
            *lock(&shared.crash) = Some(format!("The {BACKEND} core stopped: {message}"));
            shared.running.store(false, Ordering::Relaxed);
        }
        // Nobody will answer what is still queued.
        lock(&shared.commands).clear();
        if let Err(e) = core.flush_save() {
            log::error!("Failed to write save file: {e}");
        }
    });
}

fn emulate(
    core: &mut dyn Core,
    shared: &Shared,
    audio: &AudioOutput,
    hd: &SyncSender<Box<dyn HdJob>>,
) {
    let mut rgba = Vec::new();
    let mut samples = Vec::with_capacity(4096);
    let mut stretched = Vec::with_capacity(4096);
    let mut stretch = speed::TimeStretch::default();
    let mut meter = speed::SpeedMeter::new(speed::NORMAL);
    // Speed the meter was started for (0: start again).
    let mut metered = 0;

    let mut next = Instant::now();
    let mut last_frame = Instant::now();
    let mut idle = 0_u64;
    // Resolution / xBRZ of the last published frame.
    let mut published = shared.render.load(Ordering::Relaxed);
    shared.frame_taken.store(true, Ordering::Release);
    let (mut fps_start, mut fps_frames) = (Instant::now(), 0_u32);

    while shared.running.load(Ordering::Relaxed) {
        let commands = std::mem::take(&mut *lock(&shared.commands));
        for (command, reply) in commands {
            let loaded_state = matches!(command, Command::LoadState(_));
            let result = match command {
                Command::SaveState(path) => core.save_state(&path).map(|()| String::new()),
                Command::LoadState(path) => core.load_state(&path).map(|()| String::new()),
                Command::FlushSave => core.flush_save().map(|()| String::new()),
                Command::SetLink(link) => core.set_link(link).map(|()| String::new()),
                Command::WifiStatus => Ok(core.wifi_status().join(
                    "
",
                )),
            };
            let _ = reply.send(result);
            // A loaded state is shown even while paused.
            if loaded_state {
                publish_frame(core, shared, hd, true, &mut rgba);
                shared.frame_seq.fetch_add(1, Ordering::Release);
            }
        }
        if shared.paused.load(Ordering::Relaxed) && !shared.step.swap(false, Ordering::Relaxed) {
            idle += 1;
            if idle.is_multiple_of(DEBUG_EVERY) {
                shared.draw_debug(core);
            }
            // Video settings changed while paused: the paused frame again,
            // the new way (it would wait for Resume otherwise).
            let render = shared.render.load(Ordering::Relaxed);
            if render != published {
                core.set_render_scale((render & 0xFF).clamp(1, 4) as usize, render & 0x100 != 0);
                publish_frame(core, shared, hd, true, &mut rgba);
                shared.frame_seq.fetch_add(1, Ordering::Release);
                published = render;
            }
            std::thread::sleep(Duration::from_millis(10));
            next = Instant::now();
            metered = 0;
            continue;
        }
        let unlimited = shared.unlimited.load(Ordering::Relaxed);
        let percent = speed::valid(shared.speed.load(Ordering::Relaxed) as u16);
        // On a LAN link, no faster than the link sustains (melon_egui's
        // pacing): slightly slow and connected beats full speed and dropped.
        let fps = speed::fps(percent);
        let fps = core.link_rate().map_or(fps, |link| link.min(fps));
        let frame_time = Duration::from_secs_f64(1.0 / fps);
        if percent != metered {
            meter.reset(percent);
            stretch.reset();
            metered = percent;
        }
        let render = shared.render.load(Ordering::Relaxed);
        core.set_render_scale((render & 0xFF).clamp(1, 4) as usize, render & 0x100 != 0);
        published = render;
        let codes = lock(&shared.cheats).take();
        if let Some(codes) = codes {
            core.set_cheats(codes);
        }

        let touch = shared.take_touch();
        core.run_frame(shared.keys.load(Ordering::Relaxed), touch);
        core.take_audio(&mut samples);

        // Fast-forwarding or stepping would only garble the sound. Other
        // speeds keep the pitch and follow the speed really reached.
        let measured = meter.tick(1);
        if !unlimited && !shared.paused.load(Ordering::Relaxed) && !samples.is_empty() {
            if percent == speed::NORMAL {
                audio.push(&samples);
            } else {
                stretch.process(&samples, speed::stretch_ratio(percent, measured), &mut stretched);
                audio.push(&stretched);
            }
        }
        samples.clear();

        // Faster than 1x, a frame the frontend has not fetched yet is not
        // replaced (upscaling frames nobody sees is wasted work); at 1x and
        // for single steps every frame is published.
        let taken = shared.frame_taken.swap(false, Ordering::AcqRel);
        let paused = shared.paused.load(Ordering::Relaxed);
        if taken || (percent <= speed::NORMAL && !unlimited) || paused {
            publish_frame(core, shared, hd, paused, &mut rgba);
        }
        let seq = shared.frame_seq.fetch_add(1, Ordering::Release);
        if seq.is_multiple_of(DEBUG_EVERY) {
            shared.draw_debug(core);
        }
        shared.push_frame_time(last_frame.elapsed().as_secs_f32());
        last_frame = Instant::now();

        fps_frames += 1;
        let elapsed = fps_start.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let fps = fps_frames as f32 / elapsed.as_secs_f32();
            shared.fps.store(fps.to_bits(), Ordering::Relaxed);
            (fps_start, fps_frames) = (Instant::now(), 0);
        }

        // Pace to ~59.83 fps; resync instead of racing after a stall.
        next += frame_time;
        let now = Instant::now();
        if unlimited {
            next = now;
        } else if next > now {
            std::thread::sleep(next - now);
        } else if now - next > frame_time * 4 {
            next = now;
        }
    }
}

/// What this GUI keeps: the instance folder, settings, GUI config, audio
/// output and the emulator thread with the running ROM.
pub struct Frontend {
    pub instance: Instance,
    pub settings: Mutex<Settings>,
    pub config: Mutex<GuiConfig>,
    pub audio: AudioOutput,
    pub shared: Arc<Shared>,
    pub emu: Mutex<Option<EmuThread>>,
    /// The running game's ROM.
    pub rom: Mutex<Option<PathBuf>>,
    /// The LAN room (`crate::lan`).
    pub lan: crate::lan::LanState,
}

impl Frontend {
    pub fn config_path(instance: &Instance) -> PathBuf {
        instance.root().join("freya_config.json")
    }

    pub fn new(instance: Instance, settings: Settings) -> Self {
        let config: GuiConfig = std::fs::read_to_string(Self::config_path(&instance))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let volume = f32::from(config.volume.min(100)) / 100.0;
        let audio = AudioOutput::new(volume);
        let shared = Arc::new(Shared::default());
        shared.set_limit_framerate(config.limit_framerate);
        Self::apply_view(&shared, &config.view);
        Self {
            instance,
            settings: Mutex::new(settings),
            config: Mutex::new(config),
            audio,
            shared,
            emu: Mutex::new(None),
            rom: Mutex::new(None),
            lan: Mutex::default(),
        }
    }

    /// Speed and internal resolution of `view` to the emulator thread.
    pub fn apply_view(shared: &Shared, view: &View) {
        shared.set_speed(view.speed);
        shared.set_render(view.render_scale(), view.xbrz);
    }

    /// The running game's enabled cheats to the emulator thread.
    pub fn load_cheats(&self) {
        let Some(rom) = lock(&self.rom).clone() else {
            return;
        };
        let list = cheats::load(&cheats::cheats_path(&self.instance, &rom));
        let on = lock(&self.config).view.cheats_enabled;
        self.shared.set_cheats(cheats::enabled_codes(&list, on));
    }

    pub fn save_config(&self, config: &GuiConfig) {
        if let Ok(text) = serde_json::to_string_pretty(config)
            && let Err(e) = std::fs::write(Self::config_path(&self.instance), text)
        {
            log::error!("Failed to write config: {e}");
        }
    }

    /// Replaces the GUI config (volume, frame limiter, speed and
    /// resolution take effect at once) and saves it.
    pub fn set_config(&self, config: GuiConfig) {
        let volume = f32::from(config.volume.min(100)) / 100.0;
        self.audio.set_volume(volume);
        self.shared.set_limit_framerate(config.limit_framerate);
        Self::apply_view(&self.shared, &config.view);
        self.save_config(&config);
        *lock(&self.config) = config;
    }

    /// Stops the running emulator (its save is written on stop).
    pub fn stop(&self) {
        let thread = lock(&self.emu).take();
        if let Some(thread) = thread {
            thread.stop();
        }
        *lock(&self.rom) = None;
        // An undo point belongs to the game that made it.
        let _ = std::fs::remove_file(self.undo_path());
        self.shared.set_paused(false);
        // The link went away with the core: the room is left too.
        self.leave_room();
    }

    /// Loads `rom` on this build's core and runs it (stopping a previous
    /// game first so its save is flushed before the new one is read).
    ///
    /// # Errors
    /// If the ROM cannot be loaded.
    pub fn start(&self, rom: &Path) -> Result<(), String> {
        self.start_linked(rom, None)
    }

    /// [`Self::start`] on a LAN link the core is built on (melonDS).
    ///
    /// # Errors
    /// If the ROM cannot be loaded.
    pub fn start_linked(
        &self,
        rom: &Path,
        link: Option<Box<dyn std::any::Any + Send>>,
    ) -> Result<(), String> {
        self.stop();

        self.instance.import_legacy_save(rom);
        let boot = Boot::new(rom, self.instance.save_path(rom), lock(&self.settings).clone());
        *lock(&boot.link) = link;
        self.run_core(rom, backend::loader(boot))?;

        let mut config = lock(&self.config);
        remember(&mut config.recent_roms, rom);
        config.selected_rom = Some(rom.to_owned());
        self.save_config(&config);
        drop(config);
        Ok(())
    }

    /// Runs the core `loader` builds, shown as `rom` (a ROM, or what a
    /// Remote Desktop client is watching). Call [`Self::stop`] first.
    ///
    /// # Errors
    /// If the core cannot be built.
    pub fn run_core(&self, rom: &Path, loader: Loader) -> Result<(), String> {
        let backend = BACKEND;
        let fail = |e: String| {
            log::error!("Failed to load ROM {} on {backend}: {e}", rom.display());
            format!("Cannot load {} on {backend}: {e}", file_name(rom))
        };
        let thread =
            EmuThread::start(loader, Arc::clone(&self.shared), self.audio.clone()).map_err(fail)?;
        log::info!("Started {} on {backend}", rom.display());

        *lock(&self.emu) = Some(thread);
        *lock(&self.rom) = Some(rom.to_owned());
        self.load_cheats();
        Ok(())
    }

    /// Has the emulator thread carry out `command` and waits for it.
    ///
    /// # Errors
    /// If no game is running or the core fails.
    pub fn command(&self, command: Command) -> Result<String, String> {
        self.command_within(command, COMMAND_TIMEOUT)
    }

    /// [`Self::command`], waiting at most `timeout`.
    ///
    /// # Errors
    /// If no game is running, the core fails or does not answer in time.
    pub fn command_within(&self, command: Command, timeout: Duration) -> Result<String, String> {
        if lock(&self.emu).is_none() || !self.shared.is_running() {
            return Err("No game is running".to_owned());
        }
        let (reply, result) = sync_channel(1);
        lock(&self.shared.commands).push((command, reply));
        result
            .recv_timeout(timeout)
            .unwrap_or_else(|_| Err("The emulator did not answer".to_owned()))
    }

    /// The `states` folder of this instance.
    pub fn states_dir(&self) -> PathBuf {
        self.instance.root().join("states")
    }

    /// Slot `slot`'s savestate of the running game (`None`: no game, or
    /// the core has no savestates).
    pub fn state_path(&self, slot: u8) -> Option<PathBuf> {
        let rom = lock(&self.rom).clone()?;
        BACKEND.state_path(&self.states_dir(), &rom, slot)
    }

    /// Where the state from before the last load is kept (Undo state load).
    pub fn undo_path(&self) -> PathBuf {
        self.states_dir().join(format!("undo.{}", BACKEND.label()))
    }

    /// Loads the state at `path`, first keeping the current one for Undo
    /// state load.
    ///
    /// # Errors
    /// In a LAN room, or if the core cannot load it.
    pub fn load_state(&self, path: PathBuf) -> Result<(), String> {
        self.refuse_in_room()?;
        let undo = self.undo_path();
        if let Err(e) = self.command(Command::SaveState(undo.clone())) {
            log::warn!("No undo point before loading a state: {e}");
            let _ = std::fs::remove_file(&undo);
        }
        self.command(Command::LoadState(path)).map(drop)
    }

    /// File > Undo state load: back to before the last load.
    ///
    /// # Errors
    /// In a LAN room, with nothing to undo or if the core cannot load it.
    pub fn undo_state_load(&self) -> Result<(), String> {
        self.refuse_in_room()?;
        let undo = self.undo_path();
        if !undo.is_file() {
            return Err("No state load to undo".to_owned());
        }
        self.command(Command::LoadState(undo.clone()))?;
        let _ = std::fs::remove_file(undo);
        Ok(())
    }

    /// Writes the running game's save, then copies it to `dest`.
    ///
    /// # Errors
    /// If no game is running, it has not saved yet or the copy fails.
    pub fn export_save(&self, dest: &Path) -> Result<(), String> {
        let rom = lock(&self.rom).clone().ok_or("No game is running")?;
        self.command(Command::FlushSave)?;
        let save = self.instance.save_path(&rom);
        if !save.is_file() {
            return Err(format!("{} has not saved yet", file_name(&rom)));
        }
        std::fs::copy(&save, dest)
            .map(drop)
            .map_err(|e| format!("Cannot write {}: {e}", dest.display()))
    }

    /// Applies a menu action the frontend can carry out.
    ///
    /// # Errors
    /// If a game cannot be started or a folder / process cannot be opened.
    pub fn apply(&self, action: Action) -> Result<(), String> {
        let rom = lock(&self.rom).clone();
        match action {
            Action::OpenRomPath(path) => return self.start(&path),
            Action::Eject | Action::Stop => self.stop(),
            Action::Reset => {
                if let Some(rom) = rom {
                    return self.start(&rom);
                }
            }
            Action::TogglePause => self.shared.set_paused(!self.shared.is_paused()),
            Action::FrameStep => self.shared.frame_step(),
            Action::OpenDirectory => {
                ds_common::reveal(self.instance.root()).map_err(|e| e.to_string())?;
            }
            Action::LaunchInstance => ds_common::launch_instance().map_err(|e| e.to_string())?,
            Action::ClearRecent
            | Action::SetRotation(_)
            | Action::SetGap(_)
            | Action::SetLayout(_)
            | Action::ToggleSwap
            | Action::SetSizing(_)
            | Action::ToggleIntegerScaling
            | Action::ToggleFiltering
            | Action::ToggleShowFps
            | Action::ToggleLimitFramerate
            | Action::SetSpeed(_)
            | Action::CycleSpeed
            | Action::SetRenderScale(_)
            | Action::ToggleXbrz
            | Action::ScreenSize(_) => {
                let mut config = lock(&self.config);
                let view = &mut config.view;
                match action {
                    Action::SetRotation(r) => view.rotation = r,
                    Action::SetGap(g) => view.gap = g,
                    Action::SetLayout(l) => view.layout = l,
                    Action::ToggleSwap => view.swap ^= true,
                    Action::SetSizing(s) => view.sizing = s,
                    Action::ToggleIntegerScaling => view.integer_scaling ^= true,
                    Action::ToggleFiltering => view.filtering ^= true,
                    Action::ToggleShowFps => view.show_fps ^= true,
                    Action::ScreenSize(n) => view.scale = f32::from(n),
                    Action::ClearRecent => config.recent_roms.clear(),
                    Action::ToggleLimitFramerate => {
                        config.limit_framerate ^= true;
                        self.shared.set_limit_framerate(config.limit_framerate);
                    }
                    Action::SetSpeed(p) => view.speed = speed::valid(p),
                    Action::CycleSpeed => view.speed = speed::next(view.speed),
                    Action::SetRenderScale(n) => view.render_scale = n,
                    Action::ToggleXbrz => view.xbrz ^= true,
                    _ => {}
                }
                Self::apply_view(&self.shared, &config.view);
                self.save_config(&config);
                drop(config);
            }
            Action::ToggleCheats => {
                let mut config = lock(&self.config);
                config.view.cheats_enabled ^= true;
                self.save_config(&config);
                drop(config);
                self.load_cheats();
            }
            // Dialogs and the window are the GUI's.
            Action::OpenRom | Action::ImportSavefile | Action::Quit | Action::Open(_) => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `LUNARIS_TEST_ROM` (a commercial ROM shows more than hello_world).
    fn rom() -> PathBuf {
        std::env::var_os("LUNARIS_TEST_ROM").map_or_else(
            || PathBuf::from("D:/GAME/Emulator/DS/Roms/hello_world.nds"),
            PathBuf::from,
        )
    }

    /// Boots the ROM on this build's core headlessly for ~2.5 s and checks that
    /// frames come out at roughly the DS rate, with a picture.
    #[test]
    #[ignore = "requires a ROM on disk"]
    fn boots_the_built_backend() {
        let dir = std::env::temp_dir().join(format!("freya-backend-test-{BACKEND}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let rom = rom();
        let save = dir.join(rom.with_extension("sav").file_name().expect("name"));
        let boot = Boot::new(&rom, save, Settings::default());
        let shared = Arc::new(Shared::default());
        let thread =
            EmuThread::start(backend::loader(boot), Arc::clone(&shared), AudioOutput::new(0.0))
                .expect("ROM loads");
        std::thread::sleep(Duration::from_millis(2500));

        let (seq, width, rgba) = shared.frame_since(0).expect("a frame was produced");
        assert_eq!(width, 256);
        assert_eq!(rgba.len(), 256 * 192 * 4 * 2);
        assert!(rgba.as_chunks::<4>().0.iter().any(|p| p[..3] != [0, 0, 0]), "frame has content");
        assert!((100..=200).contains(&seq), "~60 fps pacing, got {seq} frames");

        // Pause stops the frames, a step makes exactly one.
        shared.set_paused(true);
        std::thread::sleep(Duration::from_millis(100));
        let paused_at = shared.frames();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(shared.frames(), paused_at, "no frames while paused");
        shared.frame_step();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(shared.frames(), paused_at + 1, "one frame per step");

        thread.stop();
        assert!(!shared.is_running());
    }

    #[test]
    fn a_missing_rom_is_an_error_not_a_panic() {
        {
            let boot = Boot::new(
                Path::new("does/not/exist.nds"),
                std::env::temp_dir().join("freya-missing.sav"),
                Settings::default(),
            );
            let shared = Arc::new(Shared::default());
            let result =
                EmuThread::start(backend::loader(boot), Arc::clone(&shared), AudioOutput::new(0.0));
            assert!(result.is_err());
            assert!(!shared.is_running());
        }
    }
}
