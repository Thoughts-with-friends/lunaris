//! The console's own thread: boot, then frames on the wall clock.

use super::*;

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

/// The thread body: boot, then run frames on the wall clock until asked to stop.
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
    let cheat_path = crate::file::settings::Settings::redirect(cheat_dir.as_ref(), rom, "mch");
    let cheats: Vec<Cheat> = crate::file::mch::load(&cheat_path)
        .unwrap_or_default()
        .into_iter()
        .map(|cheat| cheat.to_core())
        .collect();
    if !cheats.is_empty() {
        emu.nds.set_cheats(cheats.as_slice());
        log::info!("instance2 loaded {} cheat codes from {}", cheats.len(), cheat_path.display());
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

        // A pass may owe several frames, and in Remote Desktop mode the
        // controls are arriving from the network all the while. Applying the
        // one sample read above to every frame of the batch is what the remote
        // player feels as the stylus trailing their finger, so each frame gets
        // whatever has arrived by the time it starts.
        let outcome = match &stream {
            Some(stream) => run_frames_with(&mut emu, shared, due, |emu| {
                let remote = stream.input();
                emu.nds.set_keys(local.keys | remote.keys);
                match remote.touch {
                    Some((x, y)) => emu.nds.touch(x, y),
                    None => emu.nds.release_screen(),
                }
            }),
            None => run_frames(&mut emu, shared, due),
        };
        if outcome == Outcome::Stopped {
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
pub(crate) enum Outcome {
    Continue,
    Stopped,
}

/// Run `count` frames, reporting a console that stopped part way.
pub(crate) fn run_frames(emu: &mut Emu, shared: &Shared, count: u32) -> Outcome {
    run_frames_with(emu, shared, count, |_| {})
}

/// As [`run_frames`], but `before_each` runs immediately ahead of every frame.
///
/// That hook exists for one reason: to re-read the remote player's controls, so
/// a batch of frames is not driven by one stale sample. See the call site.
pub(crate) fn run_frames_with(
    emu: &mut Emu,
    shared: &Shared,
    count: u32,
    mut before_each: impl FnMut(&mut Emu),
) -> Outcome {
    for _ in 0..count {
        before_each(emu);
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
pub(crate) fn publish(emu: &mut Emu, shared: &Shared, stream: Option<&RemoteHost>) {
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
pub(crate) fn drain_audio(emu: &mut Emu, stream: Option<&RemoteHost>) {
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

/// How many sample pairs are drained from the console in one go.
///
/// A frame produces about 800 at 48 kHz; this is generous headroom for a pass
/// that ran several frames, and bounds the allocation either way.
pub(crate) const AUDIO_DRAIN_PAIRS: usize = 8192;

/// The DS's video frame rate, as [`crate::app`] uses it.
pub(crate) const FRAME_RATE: f64 = 33_513_982.0 / 560_190.0;

/// How long the thread waits before giving up on a frame it is late for. A
/// console that has fallen further behind than this — the machine was asleep,
/// say — starts afresh rather than sprinting to catch up, which on a link
/// would flood the other console with a burst of rounds.
pub(crate) const MAX_CATCH_UP: u32 = 4;
