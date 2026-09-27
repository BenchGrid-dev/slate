//! Agent backends. Slate never calls a model API; each backend launches the
//! vendor's official binary and parses its event stream.
//! See docs/decisions/0001-bring-your-own-agent.md.

pub mod claude;
pub mod codex;

use anyhow::Result;
use std::path::Path;

/// What a backend reports while a turn runs. slash renders these.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    SessionStarted(String),
    /// A streamed fragment of assistant prose. A `Text` with the full block follows.
    TextDelta(String),
    /// Assistant prose (complete block).
    Text(String),
    /// A tool call began. `detail` is a one-line summary (the command, the path…).
    ToolStart {
        name: String,
        detail: String,
    },
    /// A tool call finished.
    ToolEnd {
        name: String,
        ok: bool,
        detail: String,
    },
    /// The turn is over.
    Done {
        ok: bool,
        summary: Option<String>,
        stats: Option<String>,
    },
    /// Something we could not interpret; shown dimmed in verbose mode.
    Other(String),
}

pub struct TurnRequest<'a> {
    pub prompt: &'a str,
    /// Session context to attach as system-prompt material.
    pub context: &'a str,
    pub cwd: &'a Path,
    /// Set when slated is tracking this turn: the backend must export it to
    /// subprocesses and wire hooks / the permission tool where it can.
    pub task_id: Option<&'a str>,
    /// Path to the `slate` binary for hooks and the MCP permission server.
    pub slate_bin: Option<&'a Path>,
    /// Path to `slate-desktop`, when a Wayland display is available.
    pub desktop_bin: Option<&'a Path>,
}

pub trait Backend {
    fn name(&self) -> &'static str;
    fn session_id(&self) -> Option<&str>;
    /// Forget the current session; the next turn starts fresh.
    fn reset(&mut self);
    fn model(&self) -> Option<&str>;
    /// Change the model for subsequent turns. None restores the backend default.
    fn set_model(&mut self, model: Option<String>);
    /// Run one turn, calling `on_event` as events arrive.
    fn run_turn(&mut self, req: TurnRequest<'_>, on_event: &mut dyn FnMut(Event)) -> Result<()>;
}

pub fn by_name(name: &str, cfg: &crate::config::Config) -> Option<Box<dyn Backend>> {
    match name {
        "claude" | "claude-code" => Some(Box::new(claude::ClaudeCode::new(cfg.claude.clone()))),
        "codex" => Some(Box::new(codex::Codex::new(cfg.codex.clone()))),
        _ => None,
    }
}

/// Shared helper: read a child's stdout line by line, feed a parser, forward events.
pub(crate) fn pump_lines<R: std::io::BufRead>(
    reader: R,
    mut parse: impl FnMut(&str) -> Vec<Event>,
    on_event: &mut dyn FnMut(Event),
) -> Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        for ev in parse(&line) {
            on_event(ev);
        }
    }
    Ok(())
}

/// Compact one-line description of a tool's input, used by both adapters.
pub(crate) fn summarize_input(name: &str, input: &serde_json::Value) -> String {
    let get = |k: &str| input.get(k).and_then(|v| v.as_str()).map(str::to_string);
    match name {
        "Bash" => get("command").unwrap_or_default(),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            get("file_path").unwrap_or_default()
        }
        "Glob" | "Grep" => get("pattern").unwrap_or_default(),
        "WebFetch" | "WebSearch" => get("url").or_else(|| get("query")).unwrap_or_default(),
        "Agent" | "Task" => get("description").unwrap_or_default(),
        _ => {
            let s = input.to_string();
            crate::render::truncate(&s, 80)
        }
    }
}
