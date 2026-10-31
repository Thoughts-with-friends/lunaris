//! DS video hardware.
//!
//! ```text
//!  CPU bus --> io.rs (registers), memory.rs (palette, OAM, VRAM)
//!                      |
//!     vram.rs: banks A-I mapped to BG/OBJ/texture/LCDC regions
//!                      |
//!    +-----------------+-----------------+
//!    v                                   v
//!  engine_3d/  geometry -> raster  --->  engine_2d/ (engine A, BG0 = 3D)
//!                                        engine_2d/ (engine B)
//!                                              |
//!                                              v
//!                                display/: screen output, swap, brightness
//! ```
//!
//! [`Gpu`] owns everything: both 2D engines, the 3D engine, VRAM, palettes
//! and OAM.
pub mod debug;
pub mod display;
pub mod engine_2d;
pub mod engine_3d;
mod gpu;
mod io;
mod memory;
mod threads;
pub mod vram;

pub use gpu::Gpu;
pub use threads::{DEFAULT_RENDER_THREADS, render_threads, set_render_threads};
