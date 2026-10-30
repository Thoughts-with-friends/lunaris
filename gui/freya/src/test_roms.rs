//! The ROMs the `#[ignore]`d tests run on, named in `test_roms.json` at the
//! repository root.
//!
//! That file is gitignored: where ROMs live is this machine's business and
//! never enters the repository. `test_roms.example.json` shows its shape:
//!
//! ```json
//! {
//!   "default": "path/to/a/game.nds",
//!   "boot": "path/to/hello_world.nds",
//!   "3d": "path/to/a/3d/game.nds"
//! }
//! ```
//!
//! * `default`: a game whose title screen stands still (the upscaling tests
//!   compare 1x against 2x / 4x frames of it); its folder is the ROM folder of
//!   the click-through tests.
//! * `boot`: something light enough to keep 60 fps in a debug build (the
//!   frame pacing test), such as a homebrew hello world.
//! * `3d`: a game with 3D soon after boot (melonDS's OpenGL renderer test).
//!
//! A missing `boot` or `3d` falls back to `default`.
use std::{collections::HashMap, path::PathBuf};

fn config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test_roms.json")
}

/// The ROM `test_roms.json` names `key`.
///
/// # Panics
/// If the file, the key or the ROM is missing (only `#[ignore]`d tests ask).
pub fn rom(key: &str) -> PathBuf {
    let path = config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("{}: {e} (copy test_roms.example.json and fill in your ROMs)", path.display())
    });
    let roms: HashMap<String, PathBuf> =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let rom = roms
        .get(key)
        .or_else(|| roms.get("default"))
        .unwrap_or_else(|| panic!("{}: no \"{key}\" (nor \"default\") ROM", path.display()));
    assert!(rom.is_file(), "{}: {} is not a file", path.display(), rom.display());
    rom.clone()
}
