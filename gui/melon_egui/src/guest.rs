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

use melonds::{Cheat, SCREEN_HEIGHT, SCREEN_WIDTH};

use crate::{emu::Emu, mp::Client, remote::RemoteHost};

/// How many sample pairs are drained from the console in one go.
///
/// A frame produces about 800 at 48 kHz; this is generous headroom for a pass
/// that ran several frames, and bounds the allocation either way.
const AUDIO_DRAIN_PAIRS: usize = 8192;

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

/// A one-off instruction for the second console.
///
/// # Why commands rather than direct calls
///
/// Everything the first console's menu does — reset, savestates, cheats — is a
/// `melonds` call, and every `melonds` call for this console has to happen on
/// *this console's thread*: the core is not re-entrant across threads, and the
/// UI thread is inside `run_frame` on the first console for much of a repaint.
/// So the menu posts a command and the run loop performs it between frames,
/// which is the only place it is safe.
///
/// Without this the second console's menu bar drew every entry and only the
/// ones that happen to be pure UI (screen layout, panes) did anything — the
/// rest silently acted on the *first* console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Reboot the cart, as `System ▸ Reset` does.
    Reset,
    /// Advance exactly one frame while paused.
    FrameStep,
    /// Write a savestate: a numbered slot, or an explicit path.
    SaveState(Option<u8>, Option<PathBuf>),
    /// Read one back.
    LoadState(Option<u8>, Option<PathBuf>),
    /// Take back the last [`Command::LoadState`].
    UndoStateLoad,
    /// Replace the cart's backup memory and reboot, as `File ▸ Import
    /// savefile` does.
    ImportSave(Vec<u8>),
    /// Hand the console a fresh cheat list, or an empty one when cheats are
    /// switched off.
    SetCheats(Vec<Cheat>),
    /// Write pending backup memory out now rather than on the next tick.
    FlushSave,
    /// Set the emulated real-time clock.
    SetClock(crate::emu::Clock),
    /// Stop the console for good, closing its window.
    Stop,
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
    /// Menu commands waiting to be performed between frames. A queue rather
    /// than a single slot so that two clicks in one repaint both land.
    commands: Arc<Mutex<Vec<Command>>>,
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
    /// `stream` is set in Remote Desktop mode: this console's picture and
    /// sound then go out over it, and its controls come back from it. See
    /// [`crate::remote`].
    #[expect(
        clippy::too_many_arguments,
        reason = "each is a distinct decision the caller makes once, at boot;                   gathering them into a struct would only move the same list"
    )]
    pub fn spawn(
        rom: &Path,
        save_dir: Option<PathBuf>,
        state_dir: Option<PathBuf>,
        cheat_dir: Option<PathBuf>,
        instance_id: u32,
        mp: Client,
        start_frame: u32,
        stream: Option<Arc<RemoteHost>>,
    ) -> Self {
        let input = Arc::new(Mutex::new(Input::default()));
        let output = Arc::new(Mutex::new(Output::default()));
        let commands = Arc::new(Mutex::new(Vec::new()));
        let quit = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU32::new(start_frame));

        let handle = {
            let (rom, input, output, commands, quit, paused, frames) = (
                rom.to_path_buf(),
                Arc::clone(&input),
                Arc::clone(&output),
                Arc::clone(&commands),
                Arc::clone(&quit),
                Arc::clone(&paused),
                Arc::clone(&frames),
            );
            std::thread::Builder::new()
                .name("melon_egui-instance2".to_owned())
                .spawn(move || {
                    run(RunConfig {
                        rom: &rom,
                        save_dir,
                        state_dir,
                        cheat_dir,
                        instance_id,
                        mp,
                        start_frame,
                        stream,
                        shared: &Shared { input, output, commands, quit, paused, frames },
                    });
                })
                .ok()
        };

        Self { input, output, commands, quit, paused, frames, handle }
    }

    /// Post a menu command, to be performed between frames on the console's own
    /// thread.
    ///
    /// Returns immediately: the answer — a savestate written, a boot failure —
    /// comes back through [`Self::take_note`], because the work has not
    /// happened yet when this returns.
    pub fn send(&self, command: Command) {
        if let Ok(mut commands) = self.commands.lock() {
            commands.push(command);
        }
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
    commands: Arc<Mutex<Vec<Command>>>,
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

struct RunConfig<'a> {
    rom: &'a Path,
    save_dir: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    cheat_dir: Option<PathBuf>,
    instance_id: u32,
    mp: Client,
    start_frame: u32,
    /// Present in Remote Desktop mode; see [`Guest::spawn`].
    stream: Option<Arc<RemoteHost>>,
    shared: &'a Shared,
}

/// The thread body: boot, then run frames on the wall clock until asked to stop.
fn run(config: RunConfig) {
    let RunConfig {
        rom,
        save_dir,
        state_dir,
        cheat_dir,
        instance_id,
        mp,
        start_frame,
        stream,
        shared,
    } = config;

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
    let cheat_path = crate::config::Settings::redirect(cheat_dir.as_ref(), rom, "mch");
    let cheats: Vec<Cheat> = crate::cheats::load(&cheat_path)
        .unwrap_or_default()
        .into_iter()
        .map(|cheat| cheat.to_core())
        .collect();
    if !cheats.is_empty() {
        emu.nds.set_cheats(cheats.as_slice());
        eprintln!(
            "melon_egui: instance2 loaded {} cheat codes from {}",
            cheats.len(),
            cheat_path.display()
        );
    }
    // The wireless clock's epoch is the frame count, so a console joining a
    // session already in progress has to start from its peer's.
    emu.nds.set_frame_count(start_frame);
    shared.say(format!("running from frame {start_frame}"));

    let frame_time = Duration::from_secs_f64(1.0 / FRAME_RATE);
    let mut next = Instant::now();
    let mut last_flush = Instant::now();

    // Savestate taken before the last `LoadState`, so it can be taken back.
    let mut undo: Option<Vec<u8>> = None;
    // Frames owed by `Command::FrameStep`, which is the only way a paused
    // console advances.
    let mut stepping = 0u32;

    while !shared.quit.load(Ordering::Relaxed) {
        // Between frames, which is the only safe point: every arm of this makes
        // `melonds` calls, and the console is not re-entrant.
        if perform_commands(&mut emu, shared, &mut undo, &mut stepping) == Outcome::Stopped {
            return;
        }

        if shared.paused.load(Ordering::Relaxed) && stepping == 0 {
            // Held: the other console is not running either, so there is
            // nothing to stay in step with.
            next = Instant::now();
            std::thread::sleep(Duration::from_millis(4));
            continue;
        }

        // A step is owed regardless of the clock: `Frame step` is what advances
        // a paused console, and waiting for wall time it is not accruing would
        // make the entry do nothing.
        if stepping > 0 {
            let step = std::mem::take(&mut stepping);
            if run_frames(&mut emu, shared, step) == Outcome::Stopped {
                return;
            }
            shared.frames.store(emu.nds.frame_count(), Ordering::Relaxed);
            publish(&mut emu, shared, stream.as_deref());
            drain_audio(&mut emu, stream.as_deref());
            next = Instant::now();
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

        // In Remote Desktop mode the console belongs to the remote player, so
        // their controls arrive over the network. The host's own window can
        // still press buttons — the masks are OR-ed — but the stylus is the
        // remote player's alone: two pointers fighting over one touchscreen
        // produces a stylus that jitters between them, which is worse for both
        // than one of them simply not having it.
        let local = shared.input.lock().map(|input| *input).unwrap_or_default();
        let (keys, touch) = match &stream {
            Some(stream) => {
                let remote = stream.input();
                (local.keys | remote.keys, remote.touch)
            }
            None => (local.keys, local.touch),
        };
        emu.nds.set_keys(keys);
        match touch {
            Some((x, y)) => emu.nds.touch(x, y),
            None => emu.nds.release_screen(),
        }

        if run_frames(&mut emu, shared, due) == Outcome::Stopped {
            return;
        }
        shared.frames.store(emu.nds.frame_count(), Ordering::Relaxed);
        publish(&mut emu, shared, stream.as_deref());
        drain_audio(&mut emu, stream.as_deref());

        if last_flush.elapsed() >= Duration::from_secs(1) {
            emu.flush_save();
            last_flush = Instant::now();
        }
    }
    emu.flush_save();
}

/// Whether the run loop may carry on, or the console has stopped for good.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Continue,
    Stopped,
}

/// Run `count` frames, reporting a console that stopped part way.
fn run_frames(emu: &mut Emu, shared: &Shared, count: u32) -> Outcome {
    for _ in 0..count {
        emu.nds.run_frame();
        if !emu.nds.is_running() {
            let note = emu.stop_reason().unwrap_or_else(|| "stopped".to_owned());
            shared.say(note);
            if let Ok(mut out) = shared.output.lock() {
                out.finished = true;
            }
            return Outcome::Stopped;
        }
    }
    Outcome::Continue
}

/// Perform everything the second console's menu has asked for since the last
/// pass.
///
/// The queue is drained under its lock and acted on outside it, so that a
/// savestate — which takes a moment for a large cart — does not hold up a UI
/// thread that only wants to post the next command.
fn perform_commands(
    emu: &mut Emu,
    shared: &Shared,
    undo: &mut Option<Vec<u8>>,
    stepping: &mut u32,
) -> Outcome {
    let queued: Vec<Command> = match shared.commands.lock() {
        Ok(mut commands) => std::mem::take(&mut *commands),
        Err(_) => return Outcome::Continue,
    };
    for command in queued {
        match command {
            Command::Reset => {
                emu.nds.boot();
                shared.say("reset".to_owned());
            }
            Command::FrameStep => *stepping += 1,
            Command::SaveState(slot, path) => {
                let Some(path) = state_path(emu, slot, path) else { continue };
                let mut buffer = Vec::new();
                let outcome =
                    emu.nds.save_state(&mut buffer).map_err(|e| e.to_string()).and_then(|()| {
                        std::fs::write(&path, &buffer)
                            .map_err(|e| format!("cannot write {}: {e}", path.display()))
                    });
                shared.say(match outcome {
                    Ok(()) => format!(
                        "state saved to {} ({:.1} MiB)",
                        path.display(),
                        buffer.len() as f64 / (1024.0 * 1024.0)
                    ),
                    Err(error) => format!("save state failed: {error}"),
                });
            }
            Command::LoadState(slot, path) => {
                let Some(path) = state_path(emu, slot, path) else { continue };
                // Snapshot first, so the load can be taken back — the same undo
                // the first console offers.
                let mut before = Vec::new();
                let snapshot = emu.nds.save_state(&mut before).is_ok();
                let outcome = std::fs::read(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))
                    .and_then(|buffer| emu.nds.load_state(&buffer).map_err(|e| e.to_string()));
                shared.say(match outcome {
                    Ok(()) => {
                        *undo = snapshot.then_some(before);
                        format!("state loaded from {}", path.display())
                    }
                    Err(error) => format!("load state failed: {error}"),
                });
            }
            Command::UndoStateLoad => {
                let Some(before) = undo.take() else {
                    shared.say("nothing to undo".to_owned());
                    continue;
                };
                shared.say(match emu.nds.load_state(&before) {
                    Ok(()) => "state load undone".to_owned(),
                    Err(error) => format!("undo failed: {error}"),
                });
            }
            Command::ImportSave(data) => {
                shared.say(match emu.import_save(&data) {
                    Ok(()) => "save imported; console rebooted".to_owned(),
                    Err(error) => format!("import failed: {error}"),
                });
            }
            Command::SetCheats(cheats) => emu.nds.set_cheats(cheats.as_slice()),
            Command::FlushSave => emu.flush_save(),
            Command::SetClock(clock) => emu.set_clock(clock),
            Command::Stop => {
                emu.flush_save();
                shared.say("stopped".to_owned());
                if let Ok(mut out) = shared.output.lock() {
                    out.finished = true;
                }
                return Outcome::Stopped;
            }
        }
    }
    Outcome::Continue
}

/// Where a savestate goes: the explicit path if the menu asked for one,
/// otherwise the numbered slot in this instance's own `states` directory.
fn state_path(emu: &Emu, slot: Option<u8>, path: Option<PathBuf>) -> Option<PathBuf> {
    path.or_else(|| slot.map(|slot| emu.state_path(slot)))
}

/// Hand the console's picture to everyone who wants it: the UI thread, and —
/// in Remote Desktop mode — the encoder.
///
/// Both are served from **one** read of the framebuffers. Serving them
/// separately through [`Guest::take_screens`] would have them competing for the
/// same slot, and each would get roughly every other frame.
///
/// The UI thread's copy really is a copy: the framebuffers belong to the
/// console and are overwritten as it draws, and the UI thread must never be
/// looking at one while that happens. The encoder is served in place, on this
/// thread, so it costs nothing extra.
fn publish(emu: &mut Emu, shared: &Shared, stream: Option<&RemoteHost>) {
    let Some((top, bottom)) = emu.nds.framebuffers() else {
        return;
    };
    // Encoded here, on the console's own thread. Doing it on the UI thread
    // would spend the other console's frame time on it, which is exactly the
    // frame rate loss Remote Desktop exists to remove.
    if let Some(stream) = stream {
        stream.send_frame(top, bottom);
    }
    let screens = [top.to_vec(), bottom.to_vec()];
    debug_assert_eq!(screens[0].len(), SCREEN_WIDTH * SCREEN_HEIGHT);
    if let Ok(mut out) = shared.output.lock() {
        out.screens = Some(screens);
        out.frames = emu.nds.frame_count();
    }
}

/// Take the console's audio, and stream it if anyone is listening.
///
/// Drained **whether or not** there is a stream. The core buffers what its SPU
/// produces until somebody reads it, and this console's output was never read
/// before Remote Desktop existed; leaving it unread now would be a backlog that
/// only grows. Draining and discarding costs a memcpy a frame.
fn drain_audio(emu: &mut Emu, stream: Option<&RemoteHost>) {
    let queued = emu.nds.audio_queued();
    if queued == 0 {
        return;
    }
    let mut buffer = vec![0i16; queued.min(AUDIO_DRAIN_PAIRS) * 2];
    let pairs = emu.nds.read_audio(&mut buffer);
    if let Some(stream) = stream {
        stream.send_audio(&buffer[..pairs * 2]);
    }
}
