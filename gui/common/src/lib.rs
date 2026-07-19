//! Backend-neutral helpers shared by the `lunaris` (imgui) and `lunaris-egui`
//! front ends: the savestate file container and screen framebuffer math.
//!
//! See `docs/design/egui-migration-design.md` §3.2.

pub mod cheat_map;
pub mod framebuffer;
pub mod savestate;
pub mod upscale;
