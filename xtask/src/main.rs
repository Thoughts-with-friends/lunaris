//! Build/run dispatcher for the two interchangeable GUI front ends.
//!
//! Cargo has no native syntax for a custom `--gui <name>` flag on `build`/
//! `run`, so this crate provides it: `cargo xtask <build|run> [--gui
//! egui|imgui] [cargo args...] [-- <args passed to the emulator>]` simply
//! re-invokes `cargo <build|run> -p <package for that GUI> ...`.
//!
//! `--gui` defaults to `egui`, matching the workspace's `default-members`
//! (`gui/egui`) so that plain `cargo build`/`cargo run` also default to the
//! egui front end. See `docs/design/egui-migration-design.md` §3.3.
//!
//! Registered as the `cargo xtask` alias in `.cargo/config.toml`.

use std::process::{Command, ExitCode};

#[derive(Clone, Copy)]
enum Gui {
    Egui,
    Imgui,
}

impl Gui {
    /// Workspace package implementing this GUI. See `Cargo.toml`
    /// `[workspace] members` (`gui/egui` = `lunaris-egui`, `gui/imgui` =
    /// `lunaris`, the original front end's package name, left unrenamed).
    const fn package(self) -> &'static str {
        match self {
            Gui::Egui => "lunaris-egui",
            Gui::Imgui => "lunaris",
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage: cargo xtask <build|run> [--gui egui|imgui] [cargo args...] [-- <emulator args>]"
    );
    eprintln!("       --gui defaults to `egui` when omitted");
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
    let mut cargo_args: Vec<String> = Vec::new();
    let mut emulator_args: Vec<String> = Vec::new();

    let rest: Vec<String> = args.collect();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--gui" => {
                gui = match rest.get(i + 1).map(String::as_str) {
                    Some("egui") => Gui::Egui,
                    Some("imgui") => Gui::Imgui,
                    other => {
                        eprintln!("invalid --gui value: {other:?} (expected `egui` or `imgui`)");
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
