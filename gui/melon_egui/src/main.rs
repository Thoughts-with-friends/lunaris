// Suppress the console window that Windows otherwise attaches to a distributed
// build, matching `gui/egui` and `gui/imgui`. Left on for ordinary builds so
// that `--selftest` and `--shot` still have somewhere to print.
#![cfg_attr(feature = "release", windows_subsystem = "windows")]

//! egui front end for the melonDS core.
//!
//! # Why this crate exists
//!
//! melonDS's own front end is Qt-based, which is awkward to build on Windows.
//! This crate drives the same emulator core through the [`melonds`] bindings
//! (`melonds-rs`, GPL-3.0-or-later, as is lunaris) and renders it with egui,
//! giving a known-good reference picture to compare lunaris's own renderer and
//! wifi traces against.
//!
//! # Building
//!
//! The `melonds` feature is off by default: enabling it compiles the melonDS
//! C++ core, and `melonds-sys` drives that build with **clang targeting the
//! MSVC ABI** (melonDS is GCC/Clang code that `cl.exe` cannot compile). On
//! Windows its build script therefore needs an LLVM install, and `bindgen`
//! needs the matching `libclang.dll`:
//!
//! ```text
//! $env:LLVM_ROOT     = "<VS>/VC/Tools/Llvm/x64"      # holds bin/clang.exe
//! $env:LIBCLANG_PATH = "$env:LLVM_ROOT/bin"
//! cargo run -p melon_egui --features melonds --release
//! ```
//!
//! Visual Studio's bundled LLVM component satisfies both; a standalone
//! `winget install LLVM.LLVM` does too. `cargo xtask run --gui melon` fills
//! these in automatically by locating Visual Studio with `vswhere`.
//!
//! # A single, standalone executable
//!
//! ```text
//! cargo xtask build --gui melon --release
//! ```
//!
//! The melonDS core is linked in statically, so the result is one `.exe` that
//! needs no emulator installation, no BIOS or firmware files, and no DLLs beyond
//! what Windows itself ships: the release build additionally turns off the
//! console window and links the MSVC runtime statically, so it does not need the
//! Visual C++ redistributable either. Nothing above is required at run time --
//! LLVM is a build-time dependency only.
//!
//! # Self test
//!
//! Two headless harnesses, for two different jobs. Both fix the RTC (see
//! [`emu::use_deterministic_rtc`]) so that repeating a run shows the same frame.
//!
//! `melon_egui --selftest <rom.nds> [frames] [--dump <prefix>]` boots a cart
//! without a window, reports how much of the framebuffer is non-black, and
//! optionally writes both screens out as PNGs. **This is the one to use for
//! comparing against lunaris** (against `core/examples/dump_frame.rs`): it
//! captures the framebuffers themselves, so nothing about the window can get
//! into the picture.
//!
//! `melon_egui --shot <frames> <out.png> <rom.nds>` captures the real window
//! instead, proving that egui's texture upload and compositing work rather than
//! just the core. Note it photographs the window as it *is*, menus and hover
//! highlights included, so it is a UI check and not a source of reference
//! images.

#[cfg(feature = "melonds")]
mod app;
#[cfg(feature = "melonds")]
mod config;
#[cfg(feature = "melonds")]
mod emu;
#[cfg(feature = "melonds")]
mod menu;
#[cfg(feature = "melonds")]
mod panes;
#[cfg(feature = "melonds")]
mod selftest;
#[cfg(feature = "melonds")]
mod view;

/// Without the `melonds` feature there is no core to drive, so the binary can
/// only explain itself. Failing loudly beats a window that renders nothing.
///
/// Reachable only via `--no-default-features`, since the feature is on by
/// default.
#[cfg(not(feature = "melonds"))]
fn main() {
    eprintln!(
        "melon_egui was built with `--no-default-features`, so the `melonds` feature is off\n\
         and no emulator core is linked in. There is nothing this binary can do.\n\
         \n\
         Rebuild with the core:  cargo xtask run --gui melon --release\n\
         (this is not about the ROM -- a ROM is never required, use File > Open ROM...)"
    );
    std::process::exit(1);
}

#[cfg(feature = "melonds")]
fn main() -> eframe::Result<()> {
    let mut args = std::env::args().skip(1);
    let first = args.next();

    if first.as_deref() == Some("--selftest") {
        let rom = args.next().unwrap_or_else(|| {
            eprintln!("usage: melon_egui --selftest <rom.nds> [frames] [--dump <prefix>]");
            std::process::exit(2);
        });
        let frames = args.next().and_then(|n| n.parse().ok()).unwrap_or(600);
        let dump = (args.next().as_deref() == Some("--dump")).then(|| args.next()).flatten();
        emu::use_deterministic_rtc();
        std::process::exit(selftest::run(std::path::Path::new(&rom), frames, dump.as_deref()));
    }

    // `--shot <frames> <out.png> <rom>` runs the real window, captures it once
    // the cart has run that many frames, and exits. Unlike `--selftest` this
    // goes through egui's own texture upload and compositing, so it is what
    // proves the *window* renders rather than just the core.
    let shot = if first.as_deref() == Some("--shot") {
        let frames = args.next().and_then(|n| n.parse().ok());
        let out = args.next();
        match (frames, out) {
            (Some(frames), Some(out)) => Some((frames, std::path::PathBuf::from(out))),
            _ => {
                eprintln!("usage: melon_egui --shot <frames> <out.png> <rom.nds>");
                std::process::exit(2);
            }
        }
    } else {
        None
    };
    if shot.is_some() {
        // A capture is only worth comparing if repeating it shows the same
        // thing; see `emu::use_deterministic_rtc`.
        emu::use_deterministic_rtc();
    }

    // Any remaining argument is taken as a ROM to boot straight into, which is
    // what makes the front end scriptable from a shell.
    let rom = if shot.is_some() { args.next() } else { first }.map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(app::default_window_size())
            .with_min_inner_size(app::min_window_size())
            .with_title("melon_egui"),
        ..Default::default()
    };

    eframe::run_native(
        "melon_egui",
        options,
        Box::new(move |cc| Ok(Box::new(app::MelonEgui::new(cc, rom, shot)))),
    )
}
