//! Session context: what the agent gets told about what you have been doing.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CommandRecord {
    pub command: String,
    pub cwd: PathBuf,
    pub exit_code: Option<i32>,
}

#[derive(Debug)]
pub struct Session {
    records: VecDeque<CommandRecord>,
    cap: usize,
}

impl Session {
    pub fn new(cap: usize) -> Self {
        Self {
            records: VecDeque::new(),
            cap,
        }
    }

    pub fn push(&mut self, rec: CommandRecord) {
        if self.records.len() >= self.cap {
            self.records.pop_front();
        }
        self.records.push_back(rec);
    }

    pub fn records(&self) -> impl Iterator<Item = &CommandRecord> {
        self.records.iter()
    }

    /// Text appended to the backend's system prompt each turn.
    pub fn context_for_agent(&self, cwd: &Path) -> String {
        let mut s = String::new();
        s.push_str("You are being driven by slash, the Slate shell (https://github.com/BenchGrid-dev/slate). ");
        s.push_str("The user talks to you in natural language instead of using a shell. ");
        s.push_str("Answer first, keep command output out of the answer unless asked. ");
        s.push_str(&format!(
            "The user's current directory is {}.",
            cwd.display()
        ));
        if !self.records.is_empty() {
            s.push_str("\n\nShell commands the user ran manually in this session, oldest first (exit code in brackets):\n");
            for r in &self.records {
                let code = r
                    .exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".into());
                s.push_str(&format!(
                    "[{code}] {}  (in {})\n",
                    r.command,
                    r.cwd.display()
                ));
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_records() {
        let mut s = Session::new(2);
        for i in 0..3 {
            s.push(CommandRecord {
                command: format!("cmd{i}"),
                cwd: "/tmp".into(),
                exit_code: Some(0),
            });
        }
        let cmds: Vec<_> = s.records().map(|r| r.command.clone()).collect();
        assert_eq!(cmds, vec!["cmd1", "cmd2"]);
    }

    #[test]
    fn context_mentions_commands() {
        let mut s = Session::new(5);
        s.push(CommandRecord {
            command: "cargo build".into(),
            cwd: "/proj".into(),
            exit_code: Some(101),
        });
        let ctx = s.context_for_agent(Path::new("/proj"));
        assert!(ctx.contains("[101] cargo build"));
        assert!(ctx.contains("/proj"));
    }
}
