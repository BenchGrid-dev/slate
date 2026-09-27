//! Runs `!` lines in the user's real shell.
//!
//! Each command runs in a fresh `$SHELL -c` with the terminal inherited, so vim,
//! sudo and anything interactive just work. Working directory persists across
//! commands (the shell reports it back through a state file); environment
//! variables do not yet. Output is not captured yet either; see docs/roadmap.md.

use crate::session::CommandRecord;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct ShellRunner {
    shell: String,
    cwd: PathBuf,
}

// $1 = cwd, $2 = command, $3 = state file. Positional args avoid any quoting of the command.
const WRAPPER: &str = r#"cd -- "$1" || exit 127
eval "$2"
__slash_rc=$?
pwd -P > "$3"
exit $__slash_rc"#;

impl ShellRunner {
    pub fn new(shell: String, cwd: PathBuf) -> Self {
        Self { shell, cwd }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn set_cwd(&mut self, cwd: PathBuf) {
        self.cwd = cwd;
    }

    pub fn run(&mut self, command: &str) -> Result<CommandRecord> {
        let state = tempfile_path();
        let status = Command::new(&self.shell)
            .arg("-c")
            .arg(WRAPPER)
            .arg("slash")
            .arg(&self.cwd)
            .arg(command)
            .arg(&state)
            .env("SLASH", "1")
            .status()
            .with_context(|| format!("running {}", self.shell))?;

        let before = self.cwd.clone();
        if let Ok(p) = std::fs::read_to_string(&state) {
            let p = p.trim();
            if !p.is_empty() {
                let new = PathBuf::from(p);
                if new.is_dir() {
                    self.cwd = new;
                }
            }
        }
        let _ = std::fs::remove_file(&state);

        Ok(CommandRecord {
            command: command.to_string(),
            cwd: before,
            exit_code: status.code(),
        })
    }
}

fn tempfile_path() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("slash-state-{}-{nanos}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwd_persists_across_commands() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut r = ShellRunner::new("/bin/sh".into(), dir.path().to_path_buf());
        let rec = r.run("cd sub").unwrap();
        assert_eq!(rec.exit_code, Some(0));
        assert_eq!(r.cwd().canonicalize().unwrap(), sub.canonicalize().unwrap());
        let rec = r.run("exit 3").unwrap();
        assert_eq!(rec.exit_code, Some(3));
    }

    #[test]
    fn command_with_quotes_is_not_mangled() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = ShellRunner::new("/bin/sh".into(), dir.path().to_path_buf());
        let rec = r.run(r#"test "$(echo 'a b')" = 'a b'"#).unwrap();
        assert_eq!(rec.exit_code, Some(0));
    }
}
