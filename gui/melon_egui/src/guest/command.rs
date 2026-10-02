//! The menu commands the second console's window can send it.

use super::*;

/// A one-off instruction for the second console, queued by its menu.
///
/// Commands rather than direct calls because every `melonds` call for this
/// console must run on *its* thread, between frames (see `orders.rs`).
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
