//! What happens after the click, wherever it reaches the disk.
//!
//! The other half of the boundary [`crate::ui`] describes: a cart, a save, a
//! savestate, a cheat file, the settings. Every command that outlives the
//! session ends up in one of these.

pub mod cheats;
pub mod dialog;
pub mod mch;
pub mod picker;
pub mod report;
pub mod rom;
pub mod save;
pub mod settings;
