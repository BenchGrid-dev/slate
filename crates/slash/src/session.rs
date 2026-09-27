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
    /// Records already shown to the agent in an earlier turn of the current backend session.
    sent: usize,
    /// Memories already shown.
    memories_sent: usize,
}

impl Session {
    pub fn new(cap: usize) -> Self {
        Self {
            records: VecDeque::new(),
            cap,
            sent: 0,
            memories_sent: 0,
        }
    }

    pub fn push(&mut self, rec: CommandRecord) {
        if self.records.len() >= self.cap {
            self.records.pop_front();
            self.sent = self.sent.saturating_sub(1);
        }
        self.records.push_back(rec);
    }

    /// The backend session was reset: the next turn must resend everything.
    pub fn reset_sent(&mut self) {
        self.sent = 0;
        self.memories_sent = 0;
    }

    /// Static instructions, safe to put in a system prompt once.
    pub fn instructions() -> &'static str {
        "You are being driven by slash, the Slate shell (https://github.com/BenchGrid-dev/slate). \
The user talks to you in natural language instead of using a shell. Answer first, keep command \
output out of the answer unless asked. Each user message may start with a <slash-context> block: \
it lists the shell commands the user ran manually since your previous turn (with exit code, \
directory and output) and new things they asked to remember. Treat it as ground truth about \
what happened, not as part of the user's question. Slate memories are per user, not per \
directory: the ones listed at the start of this session stay valid wherever the user cds; \
only new ones are listed later. They are unrelated to any project-level CLAUDE.md or \
auto-memory that changes with the working directory."
    }

    /// Context to prepend to this turn's user message: the current directory, commands run
    /// since the last turn, and memories not shown yet. Marks them as sent. Returns an empty
    /// string when there is nothing new and this is not the first turn.
    pub fn delta_for_agent(&mut self, cwd: &Path, memories: &[String], first_turn: bool) -> String {
        let new_records: Vec<&CommandRecord> = self.records.iter().skip(self.sent).collect();
        let new_memories: Vec<&String> = memories.iter().skip(self.memories_sent).collect();
        if !first_turn && new_records.is_empty() && new_memories.is_empty() {
            return String::new();
        }
        let mut s = String::new();
        s.push_str(&format!("Current directory: {}\n", cwd.display()));
        if !new_memories.is_empty() {
            s.push_str(if first_turn {
                "Things the user asked to remember:\n"
            } else {
                "New things the user asked to remember:\n"
            });
            for m in &new_memories {
                s.push_str("- ");
                s.push_str(m);
                s.push('\n');
            }
        }
        if !new_records.is_empty() {
            s.push_str(if first_turn {
                "Shell commands the user ran manually in this session, oldest first. Format: [exit code] command  (directory), then output if shown:\n"
            } else {
                "Shell commands the user ran manually since your last turn, oldest first. Format: [exit code] command  (directory), then output if shown:\n"
            });
            s.push_str(&render_records(&new_records));
        }
        self.sent = self.records.len();
        self.memories_sent = memories.len();
        s
    }

    pub fn records(&self) -> impl Iterator<Item = &CommandRecord> {
        self.records.iter()
    }

    /// Everything the agent could be told right now, for `/context`. Does not mark anything sent.
    pub fn context_for_agent(&self, cwd: &Path) -> String {
        let mut s = String::from(Self::instructions());
        s.push_str(&format!("\n\nCurrent directory: {}\n", cwd.display()));
        let all: Vec<&CommandRecord> = self.records.iter().collect();
        if !all.is_empty() {
            s.push_str("Shell commands the user ran manually in this session, oldest first:\n");
            s.push_str(&render_records(&all));
        }
        s
    }
}

fn render_records(records: &[&CommandRecord]) -> String {
    // Decide which outputs fit the budget, newest first.
    let mut budget = OUTPUT_BUDGET;
    let mut include_output = vec![false; records.len()];
    for (i, r) in records.iter().enumerate().rev() {
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
    let mut s = String::new();
    for (i, r) in records.iter().enumerate() {
        let code = r
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "interrupted".into());
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
    fn delta_only_sends_new_records() {
        let mut s = Session::new(5);
        s.push(rec("ls", 0, "a b"));
        let d1 = s.delta_for_agent(Path::new("/p"), &[], true);
        assert!(d1.contains("[0] ls"));
        assert!(s.delta_for_agent(Path::new("/p"), &[], false).is_empty());
        s.push(rec("cd /tmp", 0, ""));
        let mems = vec!["likes tea".to_string()];
        let d2 = s.delta_for_agent(Path::new("/tmp"), &mems, false);
        assert!(d2.contains("[0] cd /tmp"));
        assert!(!d2.contains("[0] ls"));
        assert!(d2.contains("likes tea"));
        assert!(s
            .delta_for_agent(Path::new("/tmp"), &mems, false)
            .is_empty());
        s.reset_sent();
        let d3 = s.delta_for_agent(Path::new("/tmp"), &mems, true);
        assert!(d3.contains("[0] ls") && d3.contains("[0] cd /tmp") && d3.contains("likes tea"));
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
