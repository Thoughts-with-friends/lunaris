//! The DS cores this GUI can run, behind one [`Core`] interface.
//!
//! ```text
//!  Frontend::start(rom) --Backend::loader()--> Loader (Send closure)
//!     emulator thread: Loader() -> Box<dyn Core>
//!       every frame: run_frame(keys, touch) -> take_audio -> frame / hd_job
//!
//!  Lunaris  this repository's core (ds_emu)   HD 3D / xBRZ, states, debug, LAN
//!  MelonDs  melonds-rs (melonDS C++ core)     48 kHz audio resampled, GL hi-res
//! ```
//!
//! A core is built on the emulator thread itself (on Windows a WASAPI or GL
//! context made on the UI thread clashes with the window's COM setup).
//!
//! The backend is fixed at build time by exactly one cargo feature
//! (`lunaris` (default) or `melon`); see `gui/freya/Cargo.toml`.
#[cfg(feature = "lunaris")]
mod lunaris;
#[cfg(feature = "melon")]
pub mod melon;
#[cfg(feature = "melon")]
mod melon_gl;

#[cfg(not(any(feature = "lunaris", feature = "melon")))]
compile_error!(
    "lunaris_freya needs a backend: build with exactly one of the features \
     `lunaris` or `melon` (e.g. `--no-default-features --features melon`)"
);
#[cfg(all(feature = "lunaris", feature = "melon"))]
compile_error!(
    "lunaris_freya takes exactly one backend feature; `lunaris` is the default, \
     so add `--no-default-features` when choosing `melon` \
     (or use `cargo xtask build --gui freya --backend <name>`)"
);

use std::{
    any::Any,
    fmt,
    path::{Path, PathBuf},
};

use ds_common::Settings;

/// Which core runs the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Only the built backend is constructed.
pub enum Backend {
    Lunaris,
    MelonDs,
}

/// The backend of this build.
#[cfg(feature = "lunaris")]
pub const BACKEND: Backend = Backend::Lunaris;
#[cfg(all(feature = "melon", not(feature = "lunaris")))]
pub const BACKEND: Backend = Backend::MelonDs;

impl Backend {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Lunaris => "Lunaris",
            Self::MelonDs => "melonDS",
        }
    }

    /// Whether frames can be rendered above 1x (View > Internal resolution):
    /// Lunaris in its own HD renderer, melonDS with its OpenGL renderer (else
    /// xBRZ / pixel-doubled).
    pub const fn upscales(self) -> bool {
        matches!(self, Self::Lunaris | Self::MelonDs)
    }

    /// Whether the core draws the Debug menu's VRAM views.
    pub const fn has_debug(self) -> bool {
        matches!(self, Self::Lunaris)
    }

    /// Whether the core can save and load states.
    pub const fn has_states(self) -> bool {
        matches!(self, Self::Lunaris | Self::MelonDs)
    }

    /// Where savestate slot `slot` of `rom` is kept under the instance's
    /// `states` folder, named as the core's own GUI names it (`None`: the
    /// core has no savestates).
    ///
    /// Lunaris: `states/<rom>/state_N.bin` (as the egui GUI), melonDS:
    /// `states/<rom>.mlN` (as melonDS).
    pub fn state_path(self, states: &Path, rom: &Path, slot: u8) -> Option<PathBuf> {
        let stem = rom.file_stem()?;
        match self {
            Self::Lunaris => Some(states.join(stem).join(format!("state_{slot}.bin"))),
            Self::MelonDs => Some(states.join(format!("{}.ml{slot}", stem.to_string_lossy()))),
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A [`Loader`] that boots `boot.rom` on this build's core.
pub fn loader(boot: Boot) -> Loader {
    #[cfg(feature = "lunaris")]
    return Box::new(move || lunaris::boot(&boot));
    #[cfg(all(feature = "melon", not(feature = "lunaris")))]
    return Box::new(move || melon::boot(&boot));
}

/// What every core needs to boot a game.
#[cfg_attr(not(feature = "lunaris"), allow(dead_code))] // Not every core reads every field.
pub struct Boot {
    pub rom: PathBuf,
    /// `saves/<rom name>.sav`; the same file for every backend, so a build
    /// with another core keeps the progress.
    pub save: PathBuf,
    pub settings: Settings,
    /// A LAN link the core is built on (melonDS: `crate::lan::MelonLink`),
    /// taken by the core's boot.
    pub link: std::sync::Mutex<Option<Box<dyn Any + Send>>>,
}

impl Boot {
    pub fn new(rom: &Path, save: PathBuf, settings: Settings) -> Self {
        Self { rom: rom.to_owned(), save, settings, link: std::sync::Mutex::default() }
    }
}

/// Builds a core on the emulator thread.
pub type Loader = Box<dyn FnOnce() -> Result<Box<dyn Core>, String> + Send>;

/// An upscaled frame, rendered on the HD worker thread.
pub trait HdJob: Send {
    /// Both screens as RGBA (upper first) into `rgba`; returns the width.
    fn render(self: Box<Self>, rgba: &mut Vec<u8>) -> usize;
}

/// One running game.
pub trait Core {
    /// Emulates one frame with these buttons held (`emu_thread::key` bits)
    /// and the stylus at `touch` (DS pixels; `None` lifted).
    fn run_frame(&mut self, keys: u32, touch: Option<(i32, i32)>);

    /// Internal resolution of the frames made from now (1-4) and whether
    /// xBRZ is on (melonDS picks its renderer by both).
    fn set_render_scale(&mut self, _scale: usize, _xbrz: bool) {}

    /// Appends the frame's audio: interleaved stereo `i16` at
    /// `ds_spu::OUTPUT_RATE_HZ`.
    fn take_audio(&mut self, out: &mut Vec<i16>);

    /// Both screens at 1x as RGBA (upper first) into `rgba`.
    fn frame(&mut self, rgba: &mut Vec<u8>);

    /// The frame above 1x, for the HD worker. `None`: the core cannot
    /// upscale, show [`Self::frame`] instead.
    fn hd_job(&mut self, _scale: usize, _xbrz: bool) -> Option<Box<dyn HdJob>> {
        None
    }

    /// Replaces the Action Replay codes (empty: cheats off).
    fn set_cheats(&mut self, codes: Vec<Vec<u32>>);

    /// Writes the save file if the game changed it.
    ///
    /// # Errors
    /// If the file cannot be written.
    fn flush_save(&mut self) -> Result<(), String>;

    /// Installs (`Some`) or removes the LAN link made by `crate::lan` for
    /// this core.
    ///
    /// # Errors
    /// If the core has no LAN play or `link` is not its link type.
    fn set_link(&mut self, _link: Option<Box<dyn Any + Send>>) -> Result<(), String> {
        Err("LAN play is not supported by this core".to_owned())
    }

    /// The frame rate a LAN link sustains (`None`: no link, run at the DS
    /// rate); the emulator thread paces to it.
    fn link_rate(&self) -> Option<f64> {
        None
    }

    /// How far the emulated Wi-Fi handshake got, one stage per line.
    fn wifi_status(&self) -> Vec<String> {
        Vec::new()
    }

    /// Draws `view` (`None`: the core has no debug views).
    fn debug_image(&mut self, _view: DebugView) -> Option<Image> {
        None
    }

    /// Writes a savestate to `path`.
    ///
    /// # Errors
    /// If the core has no savestates or the file cannot be written.
    fn save_state(&mut self, _path: &Path) -> Result<(), String> {
        Err(NO_STATES.to_owned())
    }

    /// Loads the savestate at `path`.
    ///
    /// # Errors
    /// If the core has no savestates or the file cannot be loaded.
    fn load_state(&mut self, _path: &Path) -> Result<(), String> {
        Err(NO_STATES.to_owned())
    }
}

/// What a Debug view shows (the egui GUI's Palettes / Maps / Tiles / VRAM
/// windows). `engine_b`: engine B instead of A, `obj`: sprites instead of
/// backgrounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugView {
    Palettes {
        engine_b: bool,
        obj: bool,
        extended: bool,
        slot: u8,
        palette: u8,
    },
    Maps {
        engine_b: bool,
        bg: u8,
    },
    Tiles {
        engine_b: bool,
        obj: bool,
        extended: bool,
        bitmap: bool,
        bpp8: bool,
        slot: u8,
        palette: u8,
        offset: u8,
    },
    Vram {
        bank: u8,
        ignore_alpha: bool,
    },
}

/// An RGBA image and its width and height.
pub type Image = (Vec<u8>, usize, usize);

/// Why a core without savestates refuses them.
pub const NO_STATES: &str = "Savestates are not supported by this core";

/// A 1x frame (both screens, RGBA, upper first) scaled on the HD worker:
/// xBRZ, or pixels repeated (`xbrz` off), `scale` times.
#[cfg(feature = "melon")]
pub struct PostScale {
    pub rgba: Vec<u8>,
    pub scale: usize,
    pub xbrz: bool,
}

#[cfg(feature = "melon")]
impl HdJob for PostScale {
    fn render(self: Box<Self>, rgba: &mut Vec<u8>) -> usize {
        let (w, h) = (256, 192);
        let n = self.scale.clamp(1, 6);
        for screen in self.rgba.chunks_exact(w * h * 4) {
            if self.xbrz && n > 1 {
                let mut src = screen.to_vec();
                for px in src.as_chunks_mut::<4>().0 {
                    px[3] = 0xFF;
                }
                let mut out = xbrz::scale_rgba(&src, w, h, n);
                // Its edge blending is not alpha-aware.
                for px in out.as_chunks_mut::<4>().0 {
                    px[3] = 0xFF;
                }
                rgba.extend_from_slice(&out);
            } else {
                for row in screen.chunks_exact(w * 4) {
                    let start = rgba.len();
                    for px in row.as_chunks::<4>().0 {
                        for _ in 0..n {
                            rgba.extend_from_slice(px);
                        }
                    }
                    for _ in 1..n {
                        rgba.extend_from_within(start..start + w * n * 4);
                    }
                }
            }
        }
        w * n
    }
}

/// 0xAARRGGBB (lunaris and melonDS framebuffers) -> RGBA bytes appended to
/// `dst`.
pub fn push_rgba(src: &[u32], dst: &mut Vec<u8>) {
    let start = dst.len();
    dst.resize(start + src.len() * 4, 0xFF);
    for (d, &px) in dst[start..].as_chunks_mut::<4>().0.iter_mut().zip(src) {
        *d = [(px >> 16) as u8, (px >> 8) as u8, px as u8, 0xFF];
    }
}

/// The message of a panic caught while a core booted or ran.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "melon")]
    #[test]
    fn post_scale_makes_both_screens_n_times_larger() {
        for xbrz in [false, true] {
            let job = Box::new(PostScale { rgba: vec![0x80; 256 * 384 * 4], scale: 3, xbrz });
            let mut out = Vec::new();
            assert_eq!(job.render(&mut out), 768);
            assert_eq!(out.len(), 768 * 576 * 2 * 4, "xbrz {xbrz}");
        }
    }

    #[test]
    fn converts_argb_to_rgba() {
        let mut out = Vec::new();
        push_rgba(&[0xFF11_2233, 0x0044_5566], &mut out);
        assert_eq!(out, [0x11, 0x22, 0x33, 0xFF, 0x44, 0x55, 0x66, 0xFF]);
    }
}
