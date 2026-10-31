//! Which emulator core these sources are built on.
//!
//! melon_egui builds them on melonDS (`melonds`); gui/egui builds the same
//! sources on the Lunaris core (`lunaris_melonds` in place of `melonds`,
//! cargo feature `lunaris`). The UI is identical; only these names differ.
//!
//! | item              | melon_egui     | lunaris (gui/egui)            |
//! |-------------------|----------------|-------------------------------|
//! | window / app name | `melon_egui`   | `lunaris`                     |
//! | savestate slots   | `<rom>.mlN`    | `<rom>.lnN` (the core's own   |
//! |                   |                | format, so the two never mix) |

/// Window title and app id.
pub const APP_NAME: &str = if cfg!(feature = "lunaris") { "lunaris" } else { "melon_egui" };

/// Savestate slot extension prefix (`<rom>.<prefix>N`).
pub const STATE_EXT: &str = if cfg!(feature = "lunaris") { "ln" } else { "ml" };

/// Extension of slot 1, for file dialog filters.
pub const STATE_EXT1: &str = if cfg!(feature = "lunaris") { "ln1" } else { "ml1" };

/// Status bar chip naming the core.
pub const CORE_CHIP: &str = if cfg!(feature = "lunaris") { "⚙ Lunaris" } else { "⚙ melonDS" };
