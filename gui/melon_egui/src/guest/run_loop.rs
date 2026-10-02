//! The second console's thread body.
//!
//! # Flow
//!
//! ```text
//! boot (Emu::boot_mp) ─ fail ─→ report, finish
//!   │
//!   ├─ load its .mch cheats, start at the first console's frame count
//!   ▼
//! loop until Guest is dropped:
//!   1. perform queued menu commands      (orders.rs)
//!   2. paused?        → sleep 4 ms, retry
//!   3. frame step owed → run those frames now
//!   4. otherwise run as many frames as the wall clock has earned (max 4)
//!      - input: the UI's sample, or (Remote Desktop) the remote player's,
//!        re-read before every frame
//!   5. publish the picture, drain/stream the audio, flush the save each 1 s
//! ```

use super::*;

/// Everything the thread is started with. See [`Guest::spawn`].
pub(crate) struct RunConfig<'a> {
    pub(crate) rom: &'a Path,
    pub(crate) save_dir: Option<PathBuf>,
    pub(crate) state_dir: Option<PathBuf>,
    pub(crate) cheat_dir: Option<PathBuf>,
    pub(crate) instance_id: u32,
    pub(crate) mp: Client,
    pub(crate) start_frame: u32,
    /// Present in Remote Desktop mode; see [`Guest::spawn`].
    pub(crate) stream: Option<Arc<RemoteHost>>,
    pub(crate) shared: &'a Shared,
}

/// The thread body.
pub(crate) fn run(config: RunConfig) {
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
    let stream = stream.as_deref();

    let mut emu = match Emu::boot_mp(rom, save_dir.as_ref(), state_dir.as_ref(), instance_id, mp) {
        Ok(emu) => emu,
        Err(e) => return finish(shared, format!("could not boot: {e}")),
    };
    load_cheats(&mut emu, rom, cheat_dir.as_ref());
    // The wireless clock's epoch is the frame count, so a console joining a
    // session in progress has to start from its peer's.
    emu.nds.set_frame_count(start_frame);
    shared.say(format!("running from frame {start_frame}"));

    let frame_time = Duration::from_secs_f64(1.0 / FRAME_RATE);
    let mut next = Instant::now();
    let mut last_flush = Instant::now();
    // The state before the last `LoadState`, for "Undo state load".
    let mut undo: Option<Vec<u8>> = None;
    // Frames owed by `Command::FrameStep`, the only way a paused console moves.
    let mut stepping = 0u32;

    while !shared.quit.load(Ordering::Relaxed) {
        // Between frames is the only safe point for `melonds` calls.
        if perform_commands(&mut emu, shared, &mut undo, &mut stepping) == Outcome::Stopped {
            return;
        }

        if shared.paused.load(Ordering::Relaxed) && stepping == 0 {
            next = Instant::now();
            std::thread::sleep(Duration::from_millis(4));
            continue;
        }

        // A frame step runs regardless of the clock.
        if stepping > 0 {
            let step = std::mem::take(&mut stepping);
            if run_frames(&mut emu, shared, step, |_| {}) == Outcome::Stopped {
                return;
            }
            after_frames(&mut emu, shared, stream);
            next = Instant::now();
            continue;
        }

        let due = frames_due(&mut next, frame_time);
        if due == 0 {
            // Ahead of the clock: wait rather than spin. Staying level with
            // the first console in *wall* time is what lets a wireless round's
            // blocking reply collection work.
            std::thread::sleep(next.saturating_duration_since(Instant::now()).min(frame_time));
            continue;
        }

        let local = shared.input.lock().map(|input| *input).unwrap_or_default();
        let outcome = match stream {
            None => {
                emu.set_input(local.keys, local.touch);
                run_frames(&mut emu, shared, due, |_| {})
            }
            // Remote Desktop: the remote player's controls are re-read before
            // every frame so a batch is not driven by one stale sample. Keys
            // from both sides are OR-ed; the stylus is the remote player's
            // alone (two pointers on one touchscreen would make it jitter).
            Some(stream) => run_frames(&mut emu, shared, due, |emu| {
                let remote = stream.input();
                emu.set_input(local.keys | remote.keys, remote.touch);
            }),
        };
        if outcome == Outcome::Stopped {
            return;
        }
        after_frames(&mut emu, shared, stream);

        if last_flush.elapsed() >= Duration::from_secs(1) {
            emu.flush_save();
            last_flush = Instant::now();
        }
    }
    emu.flush_save();
}

/// How many frames the wall clock has earned since `next`, at most
/// [`MAX_CATCH_UP`]. A longer stall is forgotten rather than run as a burst,
/// which on a link would flood the other console with rounds.
fn frames_due(next: &mut Instant, frame_time: Duration) -> u32 {
    let now = Instant::now();
    let mut due = 0;
    while *next <= now && due < MAX_CATCH_UP {
        *next += frame_time;
        due += 1;
    }
    if due > 0 && *next < now {
        *next = now;
    }
    due
}

/// Read this console's own `.mch` and install its codes.
fn load_cheats(emu: &mut Emu, rom: &Path, cheat_dir: Option<&PathBuf>) {
    let path = crate::file::settings::Settings::redirect(cheat_dir, rom, "mch");
    let cheats: Vec<Cheat> = crate::file::mch::load(&path)
        .unwrap_or_default()
        .iter()
        .map(crate::file::mch::Cheat::to_core)
        .collect();
    if !cheats.is_empty() {
        emu.nds.set_cheats(&cheats);
        log::info!("instance2 loaded {} cheat codes from {}", cheats.len(), path.display());
    }
}

/// Whether the run loop may carry on, or the console has stopped for good.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Continue,
    Stopped,
}

/// Report `note` and mark the console as finished, so the UI closes its window.
pub(crate) fn finish(shared: &Shared, note: String) {
    shared.say(note);
    if let Ok(mut out) = shared.output.lock() {
        out.finished = true;
    }
}

/// Run `count` frames, calling `before_each` ahead of every one, and stop
/// early (reporting why) if the console stops.
fn run_frames(
    emu: &mut Emu,
    shared: &Shared,
    count: u32,
    mut before_each: impl FnMut(&mut Emu),
) -> Outcome {
    for _ in 0..count {
        before_each(emu);
        if let Err(note) = emu.run_frame_checked() {
            finish(shared, note);
            return Outcome::Stopped;
        }
    }
    Outcome::Continue
}

/// What follows every batch of frames: publish the frame count and picture,
/// and take the audio.
fn after_frames(emu: &mut Emu, shared: &Shared, stream: Option<&RemoteHost>) {
    shared.frames.store(emu.nds.frame_count(), Ordering::Relaxed);
    publish(emu, shared, stream);
    // Drained even without a stream: the core buffers its output until read,
    // so leaving it would be a backlog that only grows.
    let samples = emu.drain_audio(AUDIO_DRAIN_PAIRS);
    if let Some(stream) = stream
        && !samples.is_empty()
    {
        stream.send_audio(&samples);
    }
}

/// Hand the picture to the UI thread and, in Remote Desktop mode, to the
/// encoder — both from **one** read of the framebuffers, so neither misses
/// frames to the other. The encoder runs here, on this thread, so the first
/// console's frame time is never spent on it.
fn publish(emu: &mut Emu, shared: &Shared, stream: Option<&RemoteHost>) {
    let Some((top, bottom)) = emu.nds.framebuffers() else {
        return;
    };
    if let Some(stream) = stream {
        stream.send_frame(top, bottom);
    }
    // A copy: the core keeps drawing into its own buffers.
    let screens = [top.to_vec(), bottom.to_vec()];
    debug_assert_eq!(screens[0].len(), SCREEN_WIDTH * SCREEN_HEIGHT);
    if let Ok(mut out) = shared.output.lock() {
        out.screens = Some(screens);
        out.frames = emu.nds.frame_count();
    }
}

/// The most sample pairs drained in one go (a frame is ~800 at 48 kHz).
const AUDIO_DRAIN_PAIRS: usize = 8192;

/// The DS's video frame rate.
const FRAME_RATE: f64 = crate::emu::FRAME_RATE;

/// The most frames one pass runs to catch up with the clock.
const MAX_CATCH_UP: u32 = 4;
