//! Build/run dispatcher for the interchangeable GUI front ends.
//!
//! Cargo has no native syntax for a custom `--gui <name>` flag on `build`/
//! `run`, so this crate provides it: `cargo xtask <build|run> [--gui
//! egui|melon|freya] [--backend lunaris|melon] [cargo args...]
//! [-- <args passed to the emulator>]` simply re-invokes
//! `cargo <build|run> -p <package for that GUI> ...`.
//!
//! `--backend` (freya only) fixes the core of the build: it becomes
//! `--no-default-features --features <backend>`, since cargo itself has no
//! such flag. Default: `lunaris`.
//!
//! `--gui melon` and `--gui freya --backend melon` additionally locate an LLVM
//! toolchain for the melonDS C++ build; see [`export_llvm_root`].
//!
//! `--gui` defaults to `egui`, matching the workspace's `default-members`
//! (`gui/egui`) so that plain `cargo build`/`cargo run` also default to the
//! egui front end. See `docs/design/egui-migration-design.md` §3.3.
//!
//! Registered as the `cargo xtask` alias in `.cargo/config.toml`.

use std::{
    path::PathBuf,
    process::{Command, ExitCode},
};

#[derive(Clone, Copy)]
enum Gui {
    Egui,
    /// The melonDS-core reference front end, `gui/melon_egui`.
    Melon,
    /// The freya front end, `gui/freya`; its core (Lunaris or melonDS) is
    /// chosen at build time with `--backend`.
    Freya,
}

impl Gui {
    /// Workspace package implementing this GUI. See `Cargo.toml`
    /// `[workspace] members` (`gui/egui` = `lunaris`).
    const fn package(self) -> &'static str {
        match self {
            Gui::Egui => "lunaris",
            Gui::Melon => "melon_egui",
            Gui::Freya => "lunaris_freya",
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage: cargo xtask <build|run> [--gui egui|melon|freya] [--backend lunaris|melon] [cargo args...] [-- <emulator args>]"
    );
    eprintln!("       --gui defaults to `egui` when omitted");
    eprintln!("       --gui melon builds gui/melon_egui (the melonDS-core reference front end)");
    eprintln!("       --gui freya builds gui/freya; --backend picks its core (default `lunaris`)");
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(cargo_subcommand) = args.next() else {
        print_usage();
        return ExitCode::FAILURE;
    };
    if cargo_subcommand != "build" && cargo_subcommand != "run" && cargo_subcommand != "check" {
        print_usage();
        return ExitCode::FAILURE;
    }

    let mut gui = Gui::Egui;
    let mut backend: Option<&'static str> = None;
    let mut cargo_args: Vec<String> = Vec::new();
    let mut emulator_args: Vec<String> = Vec::new();

    let rest: Vec<String> = args.collect();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--gui" => {
                gui = match rest.get(i + 1).map(String::as_str) {
                    Some("egui") => Gui::Egui,
                    Some("melon") => Gui::Melon,
                    Some("freya") => Gui::Freya,
                    other => {
                        eprintln!(
                            "invalid --gui value: {other:?} (expected `egui`, `melon` or `freya`)"
                        );
                        return ExitCode::FAILURE;
                    }
                };
                i += 2;
            }
            "--backend" => {
                backend = match rest.get(i + 1).map(String::as_str) {
                    Some("lunaris") => Some("lunaris"),
                    Some("melon" | "melonds") => Some("melon"),
                    other => {
                        eprintln!(
                            "invalid --backend value: {other:?} (expected `lunaris` or `melon`)"
                        );
                        return ExitCode::FAILURE;
                    }
                };
                i += 2;
            }
            "--" => {
                emulator_args = rest[i + 1..].to_vec();
                break;
            }
            other => {
                cargo_args.push(other.to_owned());
                i += 1;
            }
        }
    }

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut cmd = Command::new(cargo);
    cmd.arg(&cargo_subcommand).arg("-p").arg(gui.package()).args(&cargo_args);
    if backend.is_some() && !matches!(gui, Gui::Freya) {
        eprintln!("--backend only applies to --gui freya");
        return ExitCode::FAILURE;
    }
    if matches!(gui, Gui::Freya) {
        let backend = backend.unwrap_or("lunaris");
        let mut features = backend.to_owned();
        // As for melon_egui: a release build has no console window.
        if cargo_args.iter().any(|arg| arg == "--release" || arg == "-r") {
            features.push_str(",release");
        }
        cmd.arg("--no-default-features").arg("--features").arg(features);
        if backend == "melon" {
            export_llvm_root(&mut cmd);
        }
    }
    if matches!(gui, Gui::Melon) {
        export_llvm_root(&mut cmd);
        // A release build of `melon_egui` is a build meant to be handed to
        // someone, so it gets the two things that make it a single file they can
        // double-click: no console window (the crate's `release` feature) and no
        // dependency on the MSVC runtime redistributable.
        if cargo_args.iter().any(|arg| arg == "--release" || arg == "-r") {
            cmd.arg("--features").arg("release");
            append_rustflag(&mut cmd, "-C target-feature=+crt-static");
        }
    }
    if !emulator_args.is_empty() {
        cmd.arg("--").args(&emulator_args);
    }

    match cmd.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("failed to invoke cargo: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Add `flag` to `RUSTFLAGS` for this invocation, keeping whatever the caller
/// had set.
///
/// Note that any change to `RUSTFLAGS` invalidates cached builds of the whole
/// dependency tree, which is why this is only done for release builds rather
/// than for every `--gui melon`.
fn append_rustflag(cmd: &mut Command, flag: &str) {
    let existing = std::env::var("RUSTFLAGS").unwrap_or_default();
    let combined =
        if existing.trim().is_empty() { flag.to_owned() } else { format!("{existing} {flag}") };
    cmd.env("RUSTFLAGS", combined);
}

/// Point `melonds-sys` at an LLVM install, unless the caller already has.
///
/// `melonds-sys` builds the melonDS C++ core with **clang** driving the MSVC
/// ABI -- melonDS is GCC/Clang code that `cl.exe` cannot compile -- and its
/// build script wants `LLVM_ROOT`, while `bindgen` separately wants
/// `LIBCLANG_PATH`. Neither is set on a stock Windows box even when a suitable
/// toolchain is installed, because Visual Studio's LLVM component is not on
/// `PATH`. Locating it here is what makes `cargo xtask run --gui melon` work
/// without a hand-written environment.
///
/// A no-op off Windows, where `melonds-sys` uses the platform compiler.
fn export_llvm_root(cmd: &mut Command) {
    if !cfg!(windows) || std::env::var_os("LLVM_ROOT").is_some() {
        return;
    }
    let Some(root) = find_llvm() else {
        eprintln!(
            "xtask: no LLVM install found. Set LLVM_ROOT (and LIBCLANG_PATH to its `bin`), \
             or install one: `winget install LLVM.LLVM`, or Visual Studio's \
             \"C++ Clang tools for Windows\" component."
        );
        return;
    };
    eprintln!("xtask: LLVM_ROOT={}", root.display());
    cmd.env("LLVM_ROOT", &root);
    if std::env::var_os("LIBCLANG_PATH").is_none() {
        cmd.env("LIBCLANG_PATH", root.join("bin"));
    }
}

/// An LLVM install root -- a directory whose `bin` holds `clang.exe` -- from
/// the standalone installer's location or from Visual Studio's bundled
/// component, which `vswhere` can locate.
fn find_llvm() -> Option<PathBuf> {
    let standalone = PathBuf::from(r"C:\Program Files\LLVM");
    if standalone.join("bin").join("clang.exe").exists() {
        return Some(standalone);
    }

    // vswhere ships at a fixed path with any Visual Studio installer, which is
    // the only reason a VS install can be found without already knowing where
    // it is.
    let vswhere =
        PathBuf::from(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
    let output = Command::new(vswhere)
        .args(["-products", "*", "-property", "installationPath"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .map(|line| PathBuf::from(line.trim()).join(r"VC\Tools\Llvm\x64"))
        .find(|root| root.join("bin").join("clang.exe").exists())
}
