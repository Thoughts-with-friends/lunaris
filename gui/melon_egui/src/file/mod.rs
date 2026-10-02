//! Everything that reaches the disk.
//!
//! | file          | what                                                    |
//! |---------------|---------------------------------------------------------|
//! | `rom.rs`      | booting a cart (File ▸ Open ROM)                        |
//! | `save.rs`     | save import, savestates, undo                           |
//! | `cheats.rs`   | the cheat list: edit, save, push to the consoles        |
//! | `mch.rs`      | melonDS's `.mch` cheat file format                      |
//! | `settings.rs` | `settings.json` and the `instances/` directory tree     |
//! | `picker.rs`   | system file dialogs on a thread of their own            |
//! | `dialog.rs`   | opening one, and acting on its answer                   |
//! | `report.rs`   | the crash report a stopped console leaves               |

pub mod cheats;
pub mod dialog;
pub mod mch;
pub mod picker;
pub mod report;
pub mod rom;
pub mod save;
pub mod settings;
