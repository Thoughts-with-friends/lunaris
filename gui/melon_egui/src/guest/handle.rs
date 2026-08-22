//! The handle the UI thread holds, and what crosses between the two.

use super::*;

/// A handle to the second console.
pub struct Guest {
    pub(crate) input: Arc<Mutex<Input>>,
    pub(crate) output: Arc<Mutex<Output>>,
    /// Menu commands waiting to be performed between frames. A queue rather
    /// than a single slot so that two clicks in one repaint both land.
    pub(crate) commands: Arc<Mutex<Vec<Command>>>,
    /// Asks the thread to wind up. Set by [`Drop`], so closing the window and
    /// dropping the handle are the same thing.
    pub(crate) quit: Arc<AtomicBool>,
    /// Mirrors the front end's own pause, so the second console stops with the
    /// first rather than running on alone.
    pub(crate) paused: Arc<AtomicBool>,
    /// Frames run, readable without taking the output lock.
    pub(crate) frames: Arc<AtomicU32>,
    pub(crate) handle: Option<JoinHandle<()>>,
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

/// What the UI thread hands down each repaint.
#[derive(Clone, Copy, Default)]
pub(crate) struct Input {
    pub(crate) keys: u32,
    /// Touch position, or `None` for the stylus lifted.
    pub(crate) touch: Option<(u16, u16)>,
}

/// What the console hands up.
#[derive(Default)]
pub(crate) struct Output {
    /// The newest picture, `[top, bottom]`. `None` until the first frame.
    pub(crate) screens: Option<[Screen; 2]>,
    /// Frames run, for the stop report.
    pub(crate) frames: u32,
    /// The last thing worth saying: a boot failure, or why it stopped.
    pub(crate) note: Option<String>,
    /// Set once the console has stopped for good.
    pub(crate) finished: bool,
}

/// The halves of [`Guest`] the thread keeps.
pub(crate) struct Shared {
    pub(crate) input: Arc<Mutex<Input>>,
    pub(crate) output: Arc<Mutex<Output>>,
    pub(crate) commands: Arc<Mutex<Vec<Command>>>,
    pub(crate) quit: Arc<AtomicBool>,
    pub(crate) paused: Arc<AtomicBool>,
    pub(crate) frames: Arc<AtomicU32>,
}

impl Shared {
    pub(crate) fn say(&self, note: String) {
        log::info!("second instance {note}");
        if let Ok(mut out) = self.output.lock() {
            out.note = Some(note);
        }
    }
}

/// One screen's worth of pixels, BGRA8888.
pub(crate) type Screen = Vec<u32>;
