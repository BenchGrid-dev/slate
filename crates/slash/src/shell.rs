//! Runs `!` lines in the user's real shell, inside a pty.
//!
//! slash sits between your terminal and the command: keystrokes are forwarded
//! to the pty, output is written to the screen and kept in a buffer so the
//! agent can see it later. Interactive programs (vim, sudo, less) work
//! because they see a real tty. Working directory and exported environment
//! variables persist across commands; aliases and shell functions do not
//! unless `shell_interactive = true` (then rc files are loaded per command).

use crate::session::CommandRecord;
use anyhow::{Context, Result};
use nix::pty::{openpty, Winsize};
use nix::sys::select::{select, FdSet};
use nix::sys::termios::{self, SetArg};
use nix::sys::time::TimeVal;
use nix::unistd::{read, write};
use std::collections::HashSet;
use std::io::{self, IsTerminal, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// How much output to keep per command (tail), in bytes.
const CAPTURE_CAP: usize = 64 * 1024;

pub struct ShellRunner {
    shell: String,
    interactive: bool,
    cwd: PathBuf,
    /// Environment keys we last imported from the shell, so unsets propagate.
    imported_env: HashSet<String>,
}

// $1 = cwd, $2 = command, $3 = cwd state file, $4 = env state file.
// Positional args mean the command is never re-quoted.
const WRAPPER: &str = r#"cd -- "$1" || exit 127
eval "$2"
__slash_rc=$?
pwd -P > "$3"
env -0 > "$4" 2>/dev/null
exit $__slash_rc"#;

const ENV_SKIP: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_", "SLASH", "COLUMNS", "LINES"];

impl ShellRunner {
    pub fn new(shell: String, interactive: bool, cwd: PathBuf) -> Self {
        Self {
            shell,
            interactive,
            cwd,
            imported_env: HashSet::new(),
        }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn set_cwd(&mut self, cwd: PathBuf) {
        self.cwd = cwd;
    }

    pub fn run(&mut self, command: &str) -> Result<CommandRecord> {
        let state_cwd = tempfile_path("cwd");
        let state_env = tempfile_path("env");

        let ws = current_winsize();
        let pty = openpty(&ws, None).context("openpty")?;
        let master = pty.master;
        let slave = pty.slave;

        let mut cmd = Command::new(&self.shell);
        if self.interactive {
            cmd.arg("-i");
        }
        cmd.arg("-c")
            .arg(WRAPPER)
            .arg("slash")
            .arg(&self.cwd)
            .arg(command)
            .arg(&state_cwd)
            .arg(&state_env)
            .env("SLASH", "1")
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        if std::env::var_os("TERM").is_none() {
            cmd.env("TERM", "xterm-256color");
        }
        // Make the pty the child's controlling terminal.
        unsafe {
            cmd.pre_exec(|| {
                nix::unistd::setsid().map_err(io::Error::other)?;
                if libc::ioctl(0, libc::TIOCSCTTY as libc::c_ulong, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("running {}", self.shell))?;
        // The Command still holds our copies of the slave fd. Drop them now, or the
        // master never reports hangup when the child exits (Linux blocks forever).
        drop(cmd);

        let stdin = io::stdin();
        let stdin_is_tty = stdin.is_terminal();
        let saved = if stdin_is_tty {
            termios::tcgetattr(stdin.as_fd()).ok()
        } else {
            None
        };
        if let Some(t) = &saved {
            let mut raw = t.clone();
            termios::cfmakeraw(&mut raw);
            let _ = termios::tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &raw);
        }

        let mut captured: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        let mut forward_stdin = stdin_is_tty;
        let mut last_ws = ws;
        let mut stdout = io::stdout().lock();

        // select() rather than poll(): macOS poll() does not work on pty masters.
        loop {
            let mut readfds = FdSet::new();
            readfds.insert(master.as_fd());
            if forward_stdin {
                readfds.insert(stdin.as_fd());
            }
            let mut timeout = TimeVal::new(0, 100_000);
            let n = match select(None, &mut readfds, None, None, &mut timeout) {
                Ok(n) => n,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(e.into()),
            };
            if n == 0 {
                // Idle tick: propagate resizes, notice exit.
                let now = current_winsize();
                if now.ws_col != last_ws.ws_col || now.ws_row != last_ws.ws_row {
                    last_ws = now;
                    unsafe {
                        libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ as libc::c_ulong, &now);
                    }
                }
                if child.try_wait()?.is_some() {
                    // Drain whatever is left, then stop.
                    while let Ok(k) = read(master.as_raw_fd(), &mut buf) {
                        if k == 0 {
                            break;
                        }
                        let _ = stdout.write_all(&buf[..k]);
                        append_capped(&mut captured, &buf[..k]);
                    }
                    break;
                }
                continue;
            }
            if readfds.contains(master.as_fd()) {
                match read(master.as_raw_fd(), &mut buf) {
                    Ok(0) => break,
                    Ok(k) => {
                        let _ = stdout.write_all(&buf[..k]);
                        let _ = stdout.flush();
                        append_capped(&mut captured, &buf[..k]);
                    }
                    Err(nix::errno::Errno::EIO) => break,
                    Err(nix::errno::Errno::EINTR) | Err(nix::errno::Errno::EAGAIN) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            if forward_stdin && readfds.contains(stdin.as_fd()) {
                match read(stdin.as_raw_fd(), &mut buf) {
                    Ok(0) => forward_stdin = false,
                    Ok(k) => {
                        let _ = write(master.as_fd(), &buf[..k]);
                    }
                    Err(nix::errno::Errno::EINTR) | Err(nix::errno::Errno::EAGAIN) => {}
                    Err(_) => forward_stdin = false,
                }
            }
        }

        if let Some(t) = &saved {
            let _ = termios::tcsetattr(stdin.as_fd(), SetArg::TCSANOW, t);
        }
        let status = child.wait()?;
        drop(master);

        let before = self.cwd.clone();
        if let Ok(p) = std::fs::read_to_string(&state_cwd) {
            let p = p.trim();
            if !p.is_empty() {
                let new = PathBuf::from(p);
                if new.is_dir() {
                    self.cwd = new;
                }
            }
        }
        let _ = std::fs::remove_file(&state_cwd);
        if let Ok(bytes) = std::fs::read(&state_env) {
            self.import_env(&bytes);
        }
        let _ = std::fs::remove_file(&state_env);

        let output_ended_without_newline = captured.last().is_some_and(|b| *b != b'\n');
        Ok(CommandRecord {
            command: command.to_string(),
            cwd: before,
            exit_code: status.code(),
            output: clean_output(&captured),
            output_ended_without_newline,
        })
    }

    /// Apply the child's final exported environment to this process, so later
    /// `!` commands and the agent backend see it.
    fn import_env(&mut self, raw: &[u8]) {
        let mut seen = HashSet::new();
        for entry in raw.split(|b| *b == 0) {
            if entry.is_empty() {
                continue;
            }
            let Ok(s) = std::str::from_utf8(entry) else {
                continue;
            };
            let Some((k, v)) = s.split_once('=') else {
                continue;
            };
            if k.is_empty() || ENV_SKIP.contains(&k) {
                continue;
            }
            seen.insert(k.to_string());
            if std::env::var(k).ok().as_deref() != Some(v) {
                std::env::set_var(k, v);
            }
        }
        for k in self.imported_env.difference(&seen) {
            std::env::remove_var(k);
        }
        self.imported_env = seen;
    }
}

fn append_capped(buf: &mut Vec<u8>, data: &[u8]) {
    buf.extend_from_slice(data);
    if buf.len() > CAPTURE_CAP {
        let cut = buf.len() - CAPTURE_CAP;
        buf.drain(..cut);
    }
}

fn current_winsize() -> Winsize {
    let mut ws = Winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let fd = io::stdout().as_raw_fd();
    unsafe {
        libc::ioctl(fd, libc::TIOCGWINSZ as libc::c_ulong, &mut ws);
    }
    if ws.ws_row == 0 || ws.ws_col == 0 {
        ws.ws_row = 24;
        ws.ws_col = 80;
    }
    ws
}

fn tempfile_path(kind: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("slash-{kind}-{}-{nanos}", std::process::id()))
}

/// Turn raw terminal output into something worth showing a model:
/// strip escape sequences, resolve carriage returns, normalise newlines.
pub fn clean_output(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let stripped = strip_ansi(&text).replace("\r\n", "\n");
    let mut out = String::new();
    for line in stripped.split('\n') {
        // Progress bars rewrite the line with \r; keep the final rendering.
        let last = line.rsplit('\r').next().unwrap_or("");
        out.push_str(last.trim_end());
        out.push('\n');
    }
    out.trim_end().to_string()
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            if c != '\x07' {
                out.push(c);
            }
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                // CSI: parameters, then a final byte in 0x40..=0x7E.
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                // OSC: until BEL or ESC \.
                let mut prev = '\0';
                for c in chars.by_ref() {
                    if c == '\x07' || (prev == '\x1b' && c == '\\') {
                        break;
                    }
                    prev = c;
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwd_persists_across_commands() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut r = ShellRunner::new("/bin/sh".into(), false, dir.path().to_path_buf());
        let rec = r.run("cd sub").unwrap();
        assert_eq!(rec.exit_code, Some(0));
        assert_eq!(r.cwd().canonicalize().unwrap(), sub.canonicalize().unwrap());
        let rec = r.run("exit 3").unwrap();
        assert_eq!(rec.exit_code, Some(3));
    }

    #[test]
    fn output_is_captured() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = ShellRunner::new("/bin/sh".into(), false, dir.path().to_path_buf());
        let rec = r.run("echo one; echo two >&2").unwrap();
        assert!(rec.output.contains("one"), "{:?}", rec.output);
        assert!(rec.output.contains("two"), "{:?}", rec.output);
    }

    #[test]
    fn env_persists_across_commands() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = ShellRunner::new("/bin/sh".into(), false, dir.path().to_path_buf());
        r.run("export SLASH_TEST_VAR=hello").unwrap();
        let rec = r.run("echo \"got=$SLASH_TEST_VAR\"").unwrap();
        assert!(rec.output.contains("got=hello"), "{:?}", rec.output);
        r.run("unset SLASH_TEST_VAR").unwrap();
        let rec = r.run("echo \"got=[$SLASH_TEST_VAR]\"").unwrap();
        assert!(rec.output.contains("got=[]"), "{:?}", rec.output);
    }

    #[test]
    fn command_with_quotes_is_not_mangled() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = ShellRunner::new("/bin/sh".into(), false, dir.path().to_path_buf());
        let rec = r.run(r#"test "$(echo 'a b')" = 'a b'"#).unwrap();
        assert_eq!(rec.exit_code, Some(0));
    }

    #[test]
    fn strips_ansi_and_resolves_cr() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m ok"), "red ok");
        assert_eq!(strip_ansi("\x1b]0;title\x07text"), "text");
        assert_eq!(clean_output(b"10%\r50%\r100%\r\ndone\r\n"), "100%\ndone");
    }
}
