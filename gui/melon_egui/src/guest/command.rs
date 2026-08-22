//! A one-off instruction for the second console.

use super::*;

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
