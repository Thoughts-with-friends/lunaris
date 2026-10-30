//! What the lunaris front ends (egui, freya) share on top of the core
//! crates: [`console::Console`] (one running DS with its audio), boot,
//! savestate files, cheats, config, input and screen math.

pub mod audio;
pub mod cheat_map;
pub mod config;
pub mod console;
pub mod framebuffer;
pub mod input;
pub mod loader;
pub mod log;
pub mod savestate;
pub mod upscale;
