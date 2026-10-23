//! The DS cores this GUI can run, behind one [`Core`] interface.
//!
//! ```text
//!  Frontend::start(rom) --Backend::loader()--> Loader (Send closure)
//!     emulator thread: Loader() -> Box<dyn Core>
//!       every frame: run_frame(keys, touch) -> take_audio -> frame / hd_job
//!
//!  Lunaris  nds-core (this repository)        plays its own audio, 1x only
//!  MelonDs  melonds-rs (melonDS C++ core)     48 kHz audio resampled, 1x only
//!  Cogi     cogids-rs ds_emu                  HD / xBRZ upscaling
//! ```
//!
//! A core is built on the emulator thread itself: the lunaris core opens
//! its `cpal` stream when it is created, and on Windows a WASAPI stream made
//! on the UI thread clashes with the window's COM setup.
//!
//! The backend is fixed at build time by exactly one cargo feature
//! (`lunaris` (default), `melon`, `cogi`); see `gui/freya/Cargo.toml`.
#[cfg(feature = "cogi")]
mod cogi;
#[cfg(feature = "lunaris")]
mod lunaris;
#[cfg(feature = "melon")]
mod melon;

#[cfg(not(any(feature = "lunaris", feature = "melon", feature = "cogi")))]
compile_error!(
    "lunaris_freya needs a backend: build with exactly one of the features \
     `lunaris`, `melon` or `cogi` (e.g. `--no-default-features --features melon`)"
);
#[cfg(any(
    all(feature = "lunaris", feature = "melon"),
    all(feature = "lunaris", feature = "cogi"),
    all(feature = "melon", feature = "cogi"),
))]
compile_error!(
    "lunaris_freya takes exactly one backend feature; `lunaris` is the default, \
     so add `--no-default-features` when choosing `melon` or `cogi` \
     (or use `cargo xtask build --gui freya --backend <name>`)"
);

use std::{
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
    Cogi,
}

/// The backend of this build.
#[cfg(feature = "lunaris")]
pub const BACKEND: Backend = Backend::Lunaris;
#[cfg(all(feature = "melon", not(feature = "lunaris")))]
pub const BACKEND: Backend = Backend::MelonDs;
#[cfg(all(feature = "cogi", not(any(feature = "lunaris", feature = "melon"))))]
pub const BACKEND: Backend = Backend::Cogi;

impl Backend {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Lunaris => "Lunaris",
            Self::MelonDs => "melonDS",
            Self::Cogi => "Cogi",
        }
    }

    /// Whether frames can be rendered above 1x (View > Internal resolution).
    pub const fn upscales(self) -> bool {
        matches!(self, Self::Cogi)
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
    #[cfg(all(feature = "cogi", not(any(feature = "lunaris", feature = "melon"))))]
    return Box::new(move || cogi::boot(&boot));
}

/// What every core needs to boot a game.
#[cfg_attr(not(feature = "lunaris"), allow(dead_code))] // Not every core reads every field.
pub struct Boot {
    pub rom: PathBuf,
    /// `saves/<rom name>.sav`; the same file for every backend, so a build
    /// with another core keeps the progress.
    pub save: PathBuf,
    pub settings: Settings,
    /// 0.0-1.0, for cores that play their own audio.
    pub volume: f32,
}

impl Boot {
    pub fn new(rom: &Path, save: PathBuf, settings: Settings, volume: f32) -> Self {
        Self { rom: rom.to_owned(), save, settings, volume }
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

    /// Internal resolution of the frames made from now (1-4); only cores
    /// that [`Backend::upscales`] use it.
    fn set_render_scale(&mut self, _scale: usize) {}

    /// Appends the frame's audio: interleaved stereo `i16` at
    /// `ds_spu::OUTPUT_RATE_HZ`. Cores playing their own sound add nothing.
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

    /// 0.0-1.0, for cores that play their own sound.
    fn set_volume(&mut self, _volume: f32) {}

    /// Writes the save file if the game changed it.
    ///
    /// # Errors
    /// If the file cannot be written.
    fn flush_save(&mut self) -> Result<(), String>;
}

#[cfg(any(feature = "melon", feature = "cogi", test))]
/// 0xAARRGGBB (cogi and melonDS framebuffers) -> RGBA bytes appended to
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

    #[test]
    fn converts_argb_to_rgba() {
        let mut out = Vec::new();
        push_rgba(&[0xFF11_2233, 0x0044_5566], &mut out);
        assert_eq!(out, [0x11, 0x22, 0x33, 0xFF, 0x44, 0x55, 0x66, 0xFF]);
    }
}
