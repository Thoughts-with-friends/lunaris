use std::env;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        Some("fmt") => run_fmt(),
        Some("clippy") => run_clippy(),
        Some(cmd) => {
            eprintln!("unknown xtask command: {cmd}");
            usage();
            ExitCode::FAILURE
        }
        None => {
            usage();
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!("usage: cargo xtask <fmt|clippy>");
}

fn run_fmt() -> ExitCode {
    run(Command::new("cargo").args(["fmt", "--all"]))
}

fn run_clippy() -> ExitCode {
    run(Command::new("cargo").args([
        "clippy",
        "--workspace",
        "--fix",
        "--allow-staged",
        "--allow-dirty",
    ]))
}

fn run(cmd: &mut Command) -> ExitCode {
    match cmd.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("command exited with status: {status}");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("failed to execute command: {err}");
            ExitCode::FAILURE
        }
    }
}
