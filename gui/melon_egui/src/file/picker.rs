//! System file dialogs, run off the UI thread.
//!
//! # Why not simply call `rfd::FileDialog::pick_file()`
//!
//! Both of the obvious spellings block the thread that owns the window:
//!
//! * `rfd::FileDialog::pick_file()` is synchronous by construction, and
//! * `pollster::block_on(rfd::AsyncFileDialog::…)` parks the same thread on the
//!   future.
//!
//! Under `eframe` that thread is winit's event loop. While it is parked the
//! window stops pumping messages, so it stops repainting, the emulated console
//! stops being stepped, and audio underruns — for as long as the dialog is
//! open. On Windows it is worse than cosmetic: the dialog runs its own modal
//! message loop, and a window that is not pumping its own is what the shell
//! reports as "not responding".
//!
//! So a dialog is opened on a thread of its own and its answer is collected
//! later, over a channel — the same shape [`crate::app::MelonEgui`] already
//! uses for a LAN connection being established. The emulator keeps running with
//! the dialog on screen, which is also what melonDS does.
//!
//! # What is given up
//!
//! The dialog is not parented to the emulator window, because the raw window
//! handle is not reachable from where the request is made. It therefore does
//! not centre on the window and is not modal to it: the window stays clickable
//! behind the dialog. [`Pending`] compensates for the part that matters by
//! refusing to open a second dialog while one is still open.

use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError, channel},
};

/// Which of the three system dialogs to show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// Pick an existing file.
    OpenFile,
    /// Name a file to write, confirming an overwrite.
    SaveFile,
    /// Pick a directory.
    Folder,
}

/// What to ask for.
///
/// Built by [`Request::open`], [`Request::save`] or [`Request::folder`] and
/// then refined with the builder methods, so a call site reads as one
/// expression.
#[derive(Clone)]
pub(crate) struct Request {
    kind: Kind,
    title: String,
    /// The named extension filters, in the order they are offered:
    /// `[("Nintendo DS ROM", ["nds", "dsi", "srl"])]`.
    ///
    /// A list rather than one, because a dialog that offers only the extensions
    /// a file *usually* has cannot open the one a user actually has -- a save
    /// exported under some other name is invisible in it, with no way to say
    /// "show me everything". See [`Request::any_file`].
    filters: Vec<(String, Vec<String>)>,
    /// Where the dialog opens. `None` leaves it wherever the system last was,
    /// which is the behaviour a user expects from an unconfigured dialog.
    directory: Option<PathBuf>,
    /// The name a save dialog is pre-filled with.
    file_name: Option<String>,
}

impl Request {
    /// Ask for an existing file.
    pub(crate) fn open(title: impl Into<String>) -> Self {
        Self::new(Kind::OpenFile, title)
    }

    /// Ask for a file to write.
    pub(crate) fn save(title: impl Into<String>) -> Self {
        Self::new(Kind::SaveFile, title)
    }

    /// Ask for a directory.
    pub(crate) fn folder(title: impl Into<String>) -> Self {
        Self::new(Kind::Folder, title)
    }

    fn new(kind: Kind, title: impl Into<String>) -> Self {
        Self { kind, title: title.into(), filters: Vec::new(), directory: None, file_name: None }
    }

    /// Offer `extensions` under `name`. Called more than once, each is another
    /// entry in the dialog's file-type box, the first of them selected.
    pub(crate) fn filter(mut self, name: &str, extensions: &[&str]) -> Self {
        self.filters
            .push((name.to_owned(), extensions.iter().map(|ext| (*ext).to_owned()).collect()));
        self
    }

    /// Also let anything at all be picked.
    ///
    /// For the dialogs that open a file somebody brought from elsewhere: a save
    /// exported by another emulator, or one renamed by whatever copied it, is
    /// still the file they meant, and a filter that hides it leaves them with a
    /// dialog that appears to contain nothing.
    pub(crate) fn any_file(self) -> Self {
        self.filter("all files", &["*"])
    }

    /// Open the dialog in `directory`, when there is a sensible one to offer.
    ///
    /// A path that does not exist is dropped rather than passed on: some
    /// backends fall back to the filesystem root for one, which is a worse
    /// starting point than the system's own last-used directory.
    pub(crate) fn directory(mut self, directory: Option<PathBuf>) -> Self {
        self.directory = directory.filter(|dir| dir.is_dir());
        self
    }

    /// Pre-fill a save dialog's name box.
    pub(crate) fn file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    /// Show the dialog and block until it is dismissed. Runs on the worker
    /// thread [`Pending::spawn`] starts, never on the UI thread.
    fn show(self) -> Option<PathBuf> {
        let mut dialog = rfd::AsyncFileDialog::new().set_title(&self.title);
        for (name, extensions) in &self.filters {
            let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
            dialog = dialog.add_filter(name, &extensions);
        }
        if let Some(directory) = &self.directory {
            dialog = dialog.set_directory(directory);
        }
        if let Some(file_name) = &self.file_name {
            dialog = dialog.set_file_name(file_name);
        }
        // `rfd`'s async dialogs are what the crate supports on every backend
        // this front end builds for; blocking on one *here* is fine, because
        // "here" is a thread whose only job is to wait for the user.
        pollster::block_on(async {
            match self.kind {
                Kind::OpenFile => dialog.pick_file().await.map(|file| file.path().to_path_buf()),
                Kind::SaveFile => dialog.save_file().await.map(|file| file.path().to_path_buf()),
                Kind::Folder => dialog.pick_folder().await.map(|file| file.path().to_path_buf()),
            }
        })
    }
}

/// A dialog that is open, and what its answer is for.
///
/// `P` is the caller's own "why did I open this" tag — see
/// [`crate::app::DialogPurpose`]. Keeping it here rather than in a field beside
/// the receiver is what makes it impossible to poll a dialog and forget which
/// question it was answering.
pub(crate) struct Pending<P> {
    purpose: P,
    answers: Receiver<Option<PathBuf>>,
}

impl<P> Pending<P> {
    /// Open `request` on a thread of its own, tagged with `purpose`.
    ///
    /// # Errors
    ///
    /// If the thread cannot be started. The caller should report that rather
    /// than silently doing nothing, since from the user's side the difference
    /// is a menu entry that appears to have been ignored.
    pub(crate) fn spawn(purpose: P, request: Request) -> Result<Self, String> {
        let (sender, answers) = channel();
        std::thread::Builder::new()
            .name("melon_egui-file-dialog".to_owned())
            .spawn(move || {
                // A send that fails means the app dropped the handle — the
                // window closed while the dialog was up. Nothing to report to.
                let _ = sender.send(request.show());
            })
            .map_err(|error| format!("cannot open a file dialog: {error}"))?;
        Ok(Self { purpose, answers })
    }

    /// Take the answer out of `slot` if the user has given one, emptying the
    /// slot as it does — an answered dialog is finished, and leaving the handle
    /// in place would replay its answer on every repaint.
    ///
    /// A disconnected channel counts as a cancellation: the worker only
    /// disappears without sending if it panicked, and either way there is no
    /// path to act on.
    ///
    /// Takes the slot rather than `self` so that the "still open" case can
    /// leave the handle exactly where it was, which a consuming `poll` cannot.
    pub(crate) fn take_answer(slot: &mut Option<Self>) -> Option<(P, Option<PathBuf>)> {
        let answer = match slot.as_ref()?.answers.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => None,
        };
        slot.take().map(|pending| (pending.purpose, answer))
    }
}
