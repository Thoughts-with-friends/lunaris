//! The second console, on a thread of its own.
//!
//! # Why it cannot share the first console's thread
//!
//! melonDS's local wireless is built for instances that run *concurrently*: its
//! own front end launches a second process. A wireless round is host sends CMD
//! → clients answer within microseconds of emulated time → host collects the
//! replies, and the host's collection **blocks** (melonDS `LocalMP::RecvReplies`
//! waits on a semaphore with a 25 ms timeout) precisely because the answer is
//! expected to arrive while it waits.
//!
//! Running both consoles alternately on one thread cannot satisfy that. The
//! host asks for replies in the same breath as it sends the CMD, before the
//! other console has executed a single cycle of that round; by the time the
//! client answers, the host has closed the round and reads the answer as stale.
//! That is exactly what a session produced: 10515 CMDs sent, 10512 replies sent
//! back, 281 collected, 1393 discarded as stale — and a communication error in
//! the game.
//!
//! So the second console runs here, on its own thread, paced by the same
//! wall clock as the first. Neither waits for the other's frame; they overlap,
//! and [`crate::mp`]'s blocking receives bridge the microseconds between them.
//!
//! # What crosses the boundary
//!
//! As little as possible: the keys and touch the UI thread sampled, the picture
//! the console last drew, and a note when something happens. The console itself
//! — and every `melonds` call — stays on this thread.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use melonds::{SCREEN_HEIGHT, SCREEN_WIDTH};

use crate::{emu::Emu, mp::Client};

/// The DS's video frame rate, as [`crate::app`] uses it.
const FRAME_RATE: f64 = 33_513_982.0 / 560_190.0;

/// How long the thread waits before giving up on a frame it is late for. A
/// console that has fallen further behind than this — the machine was asleep,
/// say — starts afresh rather than sprinting to catch up, which on a link
/// would flood the other console with a burst of rounds.
const MAX_CATCH_UP: u32 = 4;

/// One screen's worth of pixels, BGRA8888.
type Screen = Vec<u32>;

/// What the UI thread hands down each repaint.
#[derive(Clone, Copy, Default)]
struct Input {
    keys: u32,
    /// Touch position, or `None` for the stylus lifted.
    touch: Option<(u16, u16)>,
}

/// What the console hands up.
#[derive(Default)]
struct Output {
    /// The newest picture, `[top, bottom]`. `None` until the first frame.
    screens: Option<[Screen; 2]>,
    /// Frames run, for the stop report.
    frames: u32,
    /// The last thing worth saying: a boot failure, or why it stopped.
    note: Option<String>,
    /// Set once the console has stopped for good.
    finished: bool,
}

/// A handle to the second console.
pub struct Guest {
    input: Arc<Mutex<Input>>,
    output: Arc<Mutex<Output>>,
    /// Asks the thread to wind up. Set by [`Drop`], so closing the window and
    /// dropping the handle are the same thing.
    quit: Arc<AtomicBool>,
    /// Mirrors the front end's own pause, so the second console stops with the
    /// first rather than running on alone.
    paused: Arc<AtomicBool>,
    /// Frames run, readable without taking the output lock.
    frames: Arc<AtomicU32>,
    handle: Option<JoinHandle<()>>,
}

impl Guest {
    /// Boot `rom` as console `instance_id` on `mp`, on a thread of its own.
    ///
    /// Returns as soon as the thread is spawned: whether the cart booted is
    /// reported through [`Self::take_note`], since the answer arrives from the
    /// other thread. `start_frame` is the frame count to begin at — the wifi
    /// clock's epoch, which has to match the console it is joining (see
    /// `melonds::Nds::set_frame_count`).
    pub fn spawn(
        rom: &Path,
        save_dir: Option<PathBuf>,
        state_dir: Option<PathBuf>,
        instance_id: u32,
        mp: Client,
        start_frame: u32,
    ) -> Self {
        let input = Arc::new(Mutex::new(Input::default()));
        let output = Arc::new(Mutex::new(Output::default()));
        let quit = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU32::new(start_frame));

        let handle = {
            let (rom, input, output, quit, paused, frames) = (
                rom.to_path_buf(),
                Arc::clone(&input),
                Arc::clone(&output),
                Arc::clone(&quit),
                Arc::clone(&paused),
                Arc::clone(&frames),
            );
            std::thread::Builder::new()
                .name("melon_egui-instance2".to_owned())
                .spawn(move || {
                    run(
                        &rom,
                        save_dir,
                        state_dir,
                        instance_id,
                        mp,
                        start_frame,
                        &Shared { input, output, quit, paused, frames },
                    );
                })
                .ok()
        };

        Self { input, output, quit, paused, frames, handle }
    }

    /// Hand down this repaint's input.
    pub fn set_input(&self, keys: u32, touch: Option<(u16, u16)>) {
        if let Ok(mut input) = self.input.lock() {
            *input = Input { keys, touch };
        }
    }

    /// Run or hold, following the front end's own pause.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// The newest picture, if there is a new one since last time.
    ///
    /// Taken rather than borrowed: the console is drawing into the other half
    /// of this all the while, and holding its lock across an upload would stall
    /// it for as long as egui takes.
    pub fn take_screens(&self) -> Option<[Screen; 2]> {
        self.output.lock().ok()?.screens.take()
    }

    /// Anything the console has to say — a boot failure, a stop — once.
    pub fn take_note(&self) -> Option<String> {
        self.output.lock().ok()?.note.take()
    }

    /// Whether the console has stopped for good, so the front end can close its
    /// window rather than show a still picture forever.
    pub fn finished(&self) -> bool {
        self.output.lock().is_ok_and(|out| out.finished)
    }

    /// Frames run, for the stop report.
    pub fn frame_count(&self) -> u32 {
        self.frames.load(Ordering::Relaxed)
    }
}

impl Drop for Guest {
    /// Wind the thread up and wait for it: the console flushes its save on the
    /// way out, and a half-dropped console still holding a seat on the
    /// airwaves would keep the other one waiting for a peer that has gone.
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// The halves of [`Guest`] the thread keeps.
struct Shared {
    input: Arc<Mutex<Input>>,
    output: Arc<Mutex<Output>>,
    quit: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    frames: Arc<AtomicU32>,
}

impl Shared {
    fn say(&self, note: String) {
        eprintln!("melon_egui: second instance {note}");
        if let Ok(mut out) = self.output.lock() {
            out.note = Some(note);
        }
    }
}

/// The thread body: boot, then run frames on the wall clock until asked to stop.
fn run(
    rom: &Path,
    save_dir: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    instance_id: u32,
    mp: Client,
    start_frame: u32,
    shared: &Shared,
) {
    let mut emu = match Emu::boot_mp(rom, save_dir.as_ref(), state_dir.as_ref(), instance_id, mp) {
        Ok(emu) => emu,
        Err(e) => {
            shared.say(format!("could not boot: {e}"));
            if let Ok(mut out) = shared.output.lock() {
                out.finished = true;
            }
            return;
        }
    };
    // The wireless clock's epoch is the frame count, so a console joining a
    // session already in progress has to start from its peer's.
    emu.nds.set_frame_count(start_frame);
    shared.say(format!("running from frame {start_frame}"));

    let frame_time = Duration::from_secs_f64(1.0 / FRAME_RATE);
    let mut next = Instant::now();
    let mut last_flush = Instant::now();

    while !shared.quit.load(Ordering::Relaxed) {
        if shared.paused.load(Ordering::Relaxed) {
            // Held: the other console is not running either, so there is
            // nothing to stay in step with.
            next = Instant::now();
            std::thread::sleep(frame_time);
            continue;
        }

        // However many frames the clock has earned since the last pass, capped
        // so a long stall does not turn into a burst.
        let now = Instant::now();
        let mut due = 0;
        while next <= now && due < MAX_CATCH_UP {
            next += frame_time;
            due += 1;
        }
        if due == 0 {
            // Ahead of the clock: wait out the remainder rather than spin. This
            // is what keeps the pair overlapping in *wall* time, which is what
            // makes the host's blocking reply collection work at all.
            std::thread::sleep(next.saturating_duration_since(now).min(frame_time));
            continue;
        }
        if next < now {
            next = now;
        }

        let input = shared.input.lock().map(|input| *input).unwrap_or_default();
        emu.nds.set_keys(input.keys);
        match input.touch {
            Some((x, y)) => emu.nds.touch(x, y),
            None => emu.nds.release_screen(),
        }

        for _ in 0..due {
            emu.nds.run_frame();
            if !emu.nds.is_running() {
                let note = emu.stop_reason().unwrap_or_else(|| "stopped".to_owned());
                shared.say(note);
                if let Ok(mut out) = shared.output.lock() {
                    out.finished = true;
                }
                return;
            }
        }
        shared.frames.store(emu.nds.frame_count(), Ordering::Relaxed);
        publish(&mut emu, shared);

        if last_flush.elapsed() >= Duration::from_secs(1) {
            emu.flush_save();
            last_flush = Instant::now();
        }
    }
    emu.flush_save();
}

/// Copy the console's picture up to the UI thread.
///
/// Copied rather than shared: the framebuffers belong to the console and are
/// overwritten as it draws, and the UI thread must never be looking at one
/// while that happens.
fn publish(emu: &mut Emu, shared: &Shared) {
    let Some((top, bottom)) = emu.nds.framebuffers() else {
        return;
    };
    let screens = [top.to_vec(), bottom.to_vec()];
    debug_assert_eq!(screens[0].len(), SCREEN_WIDTH * SCREEN_HEIGHT);
    if let Ok(mut out) = shared.output.lock() {
        out.screens = Some(screens);
        out.frames = emu.nds.frame_count();
    }
}
