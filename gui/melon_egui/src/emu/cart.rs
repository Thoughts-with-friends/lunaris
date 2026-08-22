//! What a cart says about itself, and why a console stopped.

/// What the cart header says about itself, for the ROM info pane.
pub struct CartInfo {
    /// The 12-byte game title, trimmed of its padding.
    pub title: String,
    /// The 4-character game code, e.g. `IPKJ`.
    pub gamecode: String,
    /// The 2-character maker code, e.g. `01` for Nintendo.
    pub maker: String,
    /// The ROM's size on disk, in bytes.
    pub size: usize,
}

impl CartInfo {
    /// Read the fields at the start of the cart header (GBATEK, "DS Cartridge
    /// Header": title at 0, game code at 0Ch, maker code at 10h). A ROM too
    /// short to hold a header yields empty strings rather than failing, since
    /// the core has already accepted it by this point.
    pub(crate) fn parse(rom: &[u8]) -> Self {
        let field = |range: std::ops::Range<usize>| {
            rom.get(range)
                .map(|bytes| {
                    String::from_utf8_lossy(bytes).trim_end_matches(['\0', ' ']).to_owned()
                })
                .unwrap_or_default()
        };
        Self {
            title: field(0x00..0x0C),
            gamecode: field(0x0C..0x10),
            maker: field(0x10..0x12),
            size: rom.len(),
        }
    }
}

/// Why melonDS stopped a console, as `Platform::StopReason`.
///
/// The core hands one of these to the host on its way out — the only account
/// of *why* a console stopped, since `run_frame` reports the fact alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StopReason {
    /// No reason given.
    Unknown,
    /// Someone outside the console asked it to stop.
    External,
    /// The cart asked for GBA mode, which melonDS does not emulate.
    GbaModeNotSupported,
    /// The ARM9 took an exception with its vectors in memory the protection
    /// unit will not execute: a crash inside the emulated console, and the
    /// interesting case.
    BadExceptionRegion,
    /// The console shut itself down — the cart wrote the power-management
    /// chip's shutdown bit. Not a fault.
    PowerOff,
    /// A reason this build does not know about.
    Other(i32),
}

impl StopReason {
    /// melonDS `Platform::StopReason`, in its declaration order (see
    /// `Platform.h`: Unknown, External, GBAModeNotSupported,
    /// BadExceptionRegion, PowerOff).
    pub(crate) const fn from_core(reason: i32) -> Self {
        match reason {
            0 => Self::Unknown,
            1 => Self::External,
            2 => Self::GbaModeNotSupported,
            3 => Self::BadExceptionRegion,
            4 => Self::PowerOff,
            other => Self::Other(other),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "stopped for no stated reason",
            Self::External => "was stopped from outside",
            Self::GbaModeNotSupported => "asked for GBA mode, which is not emulated",
            Self::BadExceptionRegion => {
                "crashed: the ARM9 took an exception with its vectors in non-executable memory"
            }
            Self::PowerOff => "powered itself off",
            Self::Other(_) => "stopped for a reason this build does not know",
        }
    }
}
