//! Session context: what the agent gets told about what you have been doing.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CommandRecord {
    pub command: String,
    pub cwd: PathBuf,
    pub exit_code: Option<i32>,
    /// Cleaned terminal output (tail), possibly empty.
    pub output: String,
    /// The raw output did not end with a newline (so the prompt would glue on).
    pub output_ended_without_newline: bool,
}

/// Output shown per command in the agent context.
const OUTPUT_LINES: usize = 30;
const OUTPUT_CHARS: usize = 1500;
/// Total output budget across all commands.
const OUTPUT_BUDGET: usize = 6000;

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
        if self.records.is_empty() {
            return s;
        }

        // Decide which outputs fit the budget, newest first.
        let mut budget = OUTPUT_BUDGET;
        let mut include_output = vec![false; self.records.len()];
        for (i, r) in self.records.iter().enumerate().rev() {
            let tail = output_tail(&r.output);
            if tail.is_empty() {
                continue;
            }
            if tail.len() <= budget {
                budget -= tail.len();
                include_output[i] = true;
            } else {
                break;
            }
        }

        s.push_str("\n\nShell commands the user ran manually in this session, oldest first. Format: [exit code] command  (directory), then output if shown:\n");
        for (i, r) in self.records.iter().enumerate() {
            let code = r
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into());
            s.push_str(&format!(
                "\n[{code}] {}  (in {})\n",
                r.command,
                r.cwd.display()
            ));
            if include_output[i] {
                for line in output_tail(&r.output).lines() {
                    s.push_str("    ");
                    s.push_str(line);
                    s.push('\n');
                }
            } else if !r.output.is_empty() {
                s.push_str("    (output omitted)\n");
            }
        }
        s
    }
}

fn output_tail(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let skipped = lines.len().saturating_sub(OUTPUT_LINES);
    let mut tail = String::new();
    if skipped > 0 {
        tail.push_str(&format!("… {skipped} earlier lines omitted\n"));
    }
    for l in &lines[skipped..] {
        tail.push_str(l);
        tail.push('\n');
    }
    if tail.len() > OUTPUT_CHARS {
        let start = tail.len() - OUTPUT_CHARS;
        let start = tail
            .char_indices()
            .map(|(i, _)| i)
            .find(|&i| i >= start)
            .unwrap_or(start);
        tail = format!("…{}", &tail[start..]);
    }
    tail
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(cmd: &str, code: i32, out: &str) -> CommandRecord {
        CommandRecord {
            command: cmd.into(),
            cwd: "/proj".into(),
            exit_code: Some(code),
            output: out.into(),
            output_ended_without_newline: false,
        }
    }

    #[test]
    fn caps_records() {
        let mut s = Session::new(2);
        for i in 0..3 {
            s.push(rec(&format!("cmd{i}"), 0, ""));
        }
        let cmds: Vec<_> = s.records().map(|r| r.command.clone()).collect();
        assert_eq!(cmds, vec!["cmd1", "cmd2"]);
    }

    #[test]
    fn context_includes_output() {
        let mut s = Session::new(5);
        s.push(rec("cargo build", 101, "error[E0308]: mismatched types"));
        let ctx = s.context_for_agent(Path::new("/proj"));
        assert!(ctx.contains("[101] cargo build"));
        assert!(ctx.contains("    error[E0308]"));
    }

    #[test]
    fn long_output_is_tailed() {
        let big: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let mut s = Session::new(5);
        s.push(rec("yes", 0, &big));
        let ctx = s.context_for_agent(Path::new("/"));
        assert!(ctx.contains("70 earlier lines omitted"));
        assert!(ctx.contains("line 99"));
        assert!(!ctx.contains("line 10\n"));
    }

    #[test]
    fn budget_drops_oldest_outputs_first() {
        // Each tail is capped at OUTPUT_CHARS; five of them exceed OUTPUT_BUDGET.
        let big: String = (0..30).map(|i| format!("{i:0>100}\n")).collect();
        let mut s = Session::new(5);
        for name in ["a", "b", "c", "d", "e"] {
            s.push(rec(name, 0, &big));
        }
        let ctx = s.context_for_agent(Path::new("/"));
        let a = ctx.find("[0] a").unwrap();
        let b = ctx.find("[0] b").unwrap();
        let e = ctx.find("[0] e").unwrap();
        assert!(ctx[a..b].contains("(output omitted)"));
        assert!(!ctx[e..].contains("(output omitted)"));
    }
}
