//! slash: the Slate shell.
//!
//! Interactive tty → agent mode. Anything else (`-c`, a script argument, piped
//! stdin) → exec the user's POSIX shell, so `$SHELL -c` from editors and tools
//! keeps working when slash is the login shell.

mod app;
mod backend;
mod config;
mod daemon;
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

    if args.iter().any(|a| a == "--serve") {
        return match app::App::new(cfg).and_then(|mut a| a.serve()) {
            Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
            Err(e) => {
                eprintln!("slash: {e:#}");
                ExitCode::from(1)
            }
        };
    }
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

    // Ctrl-C must stop the agent (the child gets the default SIGINT and dies), not slash:
    // a no-op handler here is reset to default across exec, so children still die,
    // while slash survives to end the task and show the prompt again.
    install_sigint_noop();
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

fn install_sigint_noop() {
    extern "C" fn noop(_: libc::c_int) {}
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = noop as *const () as usize;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
    }
}
