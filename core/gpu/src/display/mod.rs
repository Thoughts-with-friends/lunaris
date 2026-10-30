//! Display output: DISPSTAT / VCOUNT, POWCNT1 and the per-scanline output
//! of both engines to the two screens (display modes, master brightness).
//!
//! ```text
//!      POWCNT1 bit 15 = 1             POWCNT1 bit 15 = 0
//!    +--------------------+         +--------------------+
//!    | top:    engine A   |         | top:    engine B   |
//!    +--------------------+         +--------------------+
//!    | bottom: engine B   |         | bottom: engine A   |   (touch screen)
//!    +--------------------+         +--------------------+
//!
//!   one frame = 263 scanlines (VCOUNT 0-262), about 60 per second:
//!     VCOUNT 0..191    visible: each 2D line is drawn at its HBlank
//!     VCOUNT 192..262  VBlank: the 3D engine swaps its polygon buffers and
//!                      renders the next 3D image; affine BG start points reload
//! ```
//!
//! GBATEK: [DS Power Control](https://problemkaputt.de/gbatek.htm#dspowercontrol),
//! [LCD Dimensions and Timings](https://problemkaputt.de/gbatek.htm#lcddimensionsandtimings)
pub mod hires;
pub(crate) mod output;
pub mod regs;
