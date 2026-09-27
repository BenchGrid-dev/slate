//! slash: the Slate shell.
//!
//! Interactive tty → agent mode. Anything else (`-c`, a script argument, piped
//! stdin) → exec the user's POSIX shell, so `$SHELL -c` from editors and tools
//! keeps working when slash is the login shell.

mod app;
mod backend;
mod config;
mod render;
mod router;
mod session;
mod shell;

use std::io::IsTerminal;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let cfg = match config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("slash: {e:#}");
            config::Config::default()
        }
    };

    if wants_fallback(&args) {
        let mut cmd = Command::new(cfg.shell());
        if args.first().map(|a| a.starts_with('-')).unwrap_or(false) {
            cmd.arg("-l");
        }
        cmd.args(&args[1..]);
        let err = cmd.exec();
        eprintln!("slash: could not exec {}: {err}", cfg.shell());
        return ExitCode::from(127);
    }

    match app::App::new(cfg).and_then(|mut a| a.run()) {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(e) => {
            eprintln!("slash: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn wants_fallback(args: &[String]) -> bool {
    let rest = &args[1..];
    if rest.iter().any(|a| a == "--version" || a == "-V") {
        println!("slash {}", slate_proto::VERSION);
        std::process::exit(0);
    }
    if rest
        .iter()
        .any(|a| a == "-c" || a == "-s" || !a.starts_with('-'))
    {
        return true;
    }
    !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal())
}
