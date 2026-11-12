// Release builds on Windows: no console window at launch.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_op_in_unsafe_fn)]

mod autostart;
mod bing;
mod cache;
mod config;
mod instance;
mod menu;
mod scheduler;
mod sync;
mod tray;
mod ui;
mod updater;
mod wallpaper;

use anyhow::{Context, Result, anyhow};
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

const HELP: &str = "\
Unbing — Bing's picture of the day as your wallpaper. No telemetry.

USAGE:
    unbing [OPTIONS]

OPTIONS:
    --background   Start in the menu bar / notification area only (used at login)
    --apply        Apply today's picture, then exit. No request until a new picture is due.
    --daemon       Stay in the background without any window or icon, and change the
                   picture at each Bing publication (used at login on Linux)
    -v, --verbose  Detailed logs (stderr)
    -h, --help     Show this help
    -V, --version  Show the version

Without options: opens the window.";

#[derive(Default, PartialEq, Eq, Debug)]
enum Mode {
    #[default]
    Gui,
    Background,
    Apply,
    Daemon,
}

#[derive(Default, Debug)]
struct Args {
    mode: Mode,
    verbose: bool,
}

enum Parsed {
    Run(Args),
    Exit(ExitCode),
}

fn parse_args(args: impl Iterator<Item = String>) -> Parsed {
    let mut out = Args::default();
    for arg in args {
        match arg.as_str() {
            "--background" => out.mode = Mode::Background,
            "--apply" => out.mode = Mode::Apply,
            "--daemon" => out.mode = Mode::Daemon,
            "-v" | "--verbose" => out.verbose = true,
            "-h" | "--help" => {
                println!("{HELP}");
                return Parsed::Exit(ExitCode::SUCCESS);
            }
            "-V" | "--version" => {
                println!("unbing {}", env!("CARGO_PKG_VERSION"));
                return Parsed::Exit(ExitCode::SUCCESS);
            }
            // Added by older macOS versions when the app is opened from the Finder.
            s if s.starts_with("-psn_") => {}
            other => {
                eprintln!("unknown option: {other}\n\n{HELP}");
                return Parsed::Exit(ExitCode::from(2));
            }
        }
    }
    Parsed::Run(out)
}

fn init_logging(verbose: bool) {
    use std::io::IsTerminal;
    let default = if verbose { "unbing=debug,info" } else { "unbing=info,warn" };
    let filter = EnvFilter::try_from_env("UNBING_LOG").unwrap_or_else(|_| EnvFilter::new(default));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(std::io::stderr().is_terminal())
        .with_writer(std::io::stderr)
        .try_init();
}

fn run(args: Args) -> Result<()> {
    match args.mode {
        Mode::Apply => scheduler::run_once().map(drop).context("pass failed, current wallpaper kept"),
        Mode::Daemon => {
            let (_keep_alive, rx) = std::sync::mpsc::channel();
            scheduler::run_daemon(rx);
            Ok(())
        }
        Mode::Gui | Mode::Background => {
            let background = args.mode == Mode::Background;
            // With a tray icon, Unbing may already be running in the background.
            if cfg!(any(target_os = "macos", target_os = "windows")) {
                match instance::acquire() {
                    Some(lock) => instance::hold(lock),
                    None => {
                        if !background {
                            instance::request_show();
                        }
                        tracing::info!("Unbing is already running");
                        return Ok(());
                    }
                }
            }
            ui::run(background).map_err(|e| anyhow!("user interface: {e}"))
        }
    }
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Parsed::Run(a) => a,
        Parsed::Exit(code) => return code,
    };
    init_logging(args.verbose);
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: &[&str]) -> Option<Args> {
        match parse_args(v.iter().map(|s| s.to_string())) {
            Parsed::Run(a) => Some(a),
            Parsed::Exit(_) => None,
        }
    }

    #[test]
    fn modes_and_flags() {
        assert_eq!(parse(&[]).unwrap().mode, Mode::Gui);
        assert_eq!(parse(&["--background"]).unwrap().mode, Mode::Background);
        let a = parse(&["--apply", "--verbose"]).unwrap();
        assert_eq!(a.mode, Mode::Apply);
        assert!(a.verbose);
        assert_eq!(parse(&["--daemon"]).unwrap().mode, Mode::Daemon);
        assert!(parse(&["--nope"]).is_none());
    }
}
