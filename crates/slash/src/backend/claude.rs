//! Claude Code adapter.
//!
//! Launches `claude -p --output-format stream-json --verbose`, feeds the prompt on
//! stdin, and parses the JSONL event stream. Sessions are continued with `--resume`.
//! Session context goes in via `--append-system-prompt`.
//!
//! Permissions: until slated's approval broker exists (which will be wired in via
//! `--permission-prompt-tool`), headless Claude Code only has `--permission-mode`
//! and `--allowedTools`. Both are configurable in slash.toml.

use super::{pump_lines, summarize_input, Backend, Event, TurnRequest};
use crate::config::ClaudeConfig;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};

pub struct ClaudeCode {
    cfg: ClaudeConfig,
    session_id: Option<String>,
}

impl ClaudeCode {
    pub fn new(cfg: ClaudeConfig) -> Self {
        Self {
            cfg,
            session_id: None,
        }
    }

    fn command(&self, req: &TurnRequest<'_>) -> Command {
        let mut cmd = Command::new(&self.cfg.bin);
        cmd.arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages")
            .arg("--append-system-prompt")
            .arg(req.context)
            .current_dir(req.cwd);
        match (req.task_id, req.slate_bin) {
            (Some(task_id), Some(slate)) => {
                // slated decides: hooks classify every call, Confirm-tier calls go to
                // the permission tool, which asks the human through slash.
                let slate = slate.display().to_string();
                let settings = serde_json::json!({
                    "hooks": {
                        "PreToolUse": [{"hooks": [{"type": "command", "command": format!("{slate} hook pre-tool-use")}]}],
                        "PostToolUse": [{"hooks": [{"type": "command", "command": format!("{slate} hook post-tool-use")}]}]
                    }
                });
                let mut servers = serde_json::json!({"slate": {"command": slate, "args": ["mcp"]}});
                if let Some(d) = req.desktop_bin {
                    servers["desktop"] =
                        serde_json::json!({"command": d.display().to_string(), "args": ["serve"]});
                }
                let mcp = serde_json::json!({ "mcpServers": servers });
                cmd.env(slate_proto::ENV_TASK, task_id)
                    .arg("--permission-mode")
                    .arg("default")
                    .arg("--settings")
                    .arg(settings.to_string())
                    .arg("--mcp-config")
                    .arg(mcp.to_string())
                    .arg("--permission-prompt-tool")
                    .arg("mcp__slate__approve");
            }
            _ => {
                cmd.arg("--permission-mode").arg(&self.cfg.permission_mode);
            }
        }
        if let Some(id) = &self.session_id {
            cmd.arg("--resume").arg(id);
        }
        if let Some(m) = &self.cfg.model {
            cmd.arg("--model").arg(m);
        }
        if !self.cfg.allowed_tools.is_empty() {
            cmd.arg("--allowedTools")
                .arg(self.cfg.allowed_tools.join(","));
        }
        cmd.args(&self.cfg.extra_args);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

impl Backend for ClaudeCode {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    fn reset(&mut self) {
        self.session_id = None;
    }

    fn model(&self) -> Option<&str> {
        self.cfg.model.as_deref()
    }

    fn set_model(&mut self, model: Option<String>) {
        self.cfg.model = model;
    }

    fn run_turn(&mut self, req: TurnRequest<'_>, on_event: &mut dyn FnMut(Event)) -> Result<()> {
        let mut child = self
            .command(&req)
            .spawn()
            .with_context(|| format!("launching {} (is Claude Code installed?)", self.cfg.bin))?;

        {
            let mut stdin = child.stdin.take().context("no stdin")?;
            stdin.write_all(req.prompt.as_bytes())?;
            stdin.write_all(b"\n")?;
        }

        let stdout = child.stdout.take().context("no stdout")?;
        let mut stderr = child.stderr.take().context("no stderr")?;
        let stderr_thread = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });

        let mut session_id = self.session_id.clone();
        let mut seen_tools = std::collections::HashMap::<String, String>::new();
        pump_lines(
            BufReader::new(stdout),
            |line| parse_line(line, &mut seen_tools),
            &mut |ev| {
                if let Event::SessionStarted(id) = &ev {
                    session_id = Some(id.clone());
                }
                on_event(ev);
            },
        )?;

        let status = child.wait()?;
        let err = stderr_thread.join().unwrap_or_default();
        self.session_id = session_id;
        if !status.success() {
            bail!(
                "{} exited with {}{}",
                self.cfg.bin,
                status,
                if err.trim().is_empty() {
                    String::new()
                } else {
                    format!(":\n{}", err.trim())
                }
            );
        }
        Ok(())
    }
}

/// Parse one stream-json line into zero or more events.
/// `tools` maps tool_use ids to names so tool_result lines can be attributed.
pub fn parse_line(line: &str, tools: &mut std::collections::HashMap<String, String>) -> Vec<Event> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![Event::Other(line.to_string())];
    };
    let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
    match ty {
        "stream_event" => {
            if v.pointer("/event/type").and_then(Value::as_str) == Some("content_block_delta") {
                if let Some(t) = v.pointer("/event/delta/text").and_then(Value::as_str) {
                    return vec![Event::TextDelta(t.to_string())];
                }
                if let Some(t) = v.pointer("/event/delta/thinking").and_then(Value::as_str) {
                    return vec![Event::Thinking(t.to_string())];
                }
            }
            vec![]
        }
        "system" => {
            if v.get("subtype").and_then(Value::as_str) == Some("init") {
                if let Some(id) = v.get("session_id").and_then(Value::as_str) {
                    return vec![Event::SessionStarted(id.to_string())];
                }
            }
            vec![]
        }
        "assistant" => {
            let mut out = vec![];
            if let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) {
                for b in blocks {
                    match b.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(Value::as_str) {
                                if !t.trim().is_empty() {
                                    out.push(Event::Text(t.to_string()));
                                }
                            }
                        }
                        Some("tool_use") => {
                            let name = b
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("tool")
                                .to_string();
                            let input = b.get("input").cloned().unwrap_or(Value::Null);
                            if let Some(id) = b.get("id").and_then(Value::as_str) {
                                tools.insert(id.to_string(), name.clone());
                            }
                            out.push(Event::ToolStart {
                                detail: summarize_input(&name, &input),
                                name,
                            });
                        }
                        _ => {}
                    }
                }
            }
            out
        }
        "user" => {
            let mut out = vec![];
            if let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) {
                for b in blocks {
                    if b.get("type").and_then(Value::as_str) == Some("tool_result") {
                        let id = b.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                        let name = tools.remove(id).unwrap_or_else(|| "tool".into());
                        let is_error = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                        let detail = match b.get("content") {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Array(parts)) => parts
                                .iter()
                                .filter_map(|p| p.get("text").and_then(Value::as_str))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            _ => String::new(),
                        };
                        out.push(Event::ToolEnd {
                            name,
                            ok: !is_error,
                            detail,
                        });
                    }
                }
            }
            out
        }
        "result" => {
            let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            let subtype = v.get("subtype").and_then(Value::as_str).unwrap_or("");
            let summary = v.get("result").and_then(Value::as_str).map(str::to_string);
            let mut stats = vec![];
            if let Some(ms) = v.get("duration_ms").and_then(Value::as_u64) {
                stats.push(format!("{:.1}s", ms as f64 / 1000.0));
            }
            if let Some(n) = v.get("num_turns").and_then(Value::as_u64) {
                stats.push(format!("{n} turns"));
            }
            if is_error && !subtype.is_empty() {
                stats.push(subtype.to_string());
            }
            vec![Event::Done {
                ok: !is_error,
                summary,
                stats: if stats.is_empty() {
                    None
                } else {
                    Some(stats.join(", "))
                },
            }]
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn parses_init_session() {
        let mut t = HashMap::new();
        let ev = parse_line(
            r#"{"type":"system","subtype":"init","session_id":"abc-123","cwd":"/x","tools":[]}"#,
            &mut t,
        );
        assert_eq!(ev, vec![Event::SessionStarted("abc-123".into())]);
    }

    #[test]
    fn parses_assistant_text_and_tool_use() {
        let mut t = HashMap::new();
        let ev = parse_line(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Looking."},{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"ls -la"}}]}}"#,
            &mut t,
        );
        assert_eq!(
            ev,
            vec![
                Event::Text("Looking.".into()),
                Event::ToolStart {
                    name: "Bash".into(),
                    detail: "ls -la".into()
                }
            ]
        );
        assert_eq!(t.get("tu1").map(String::as_str), Some("Bash"));
    }

    #[test]
    fn parses_tool_result_with_attribution() {
        let mut t = HashMap::new();
        t.insert("tu1".to_string(), "Bash".to_string());
        let ev = parse_line(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":"total 0","is_error":false}]}}"#,
            &mut t,
        );
        assert_eq!(
            ev,
            vec![Event::ToolEnd {
                name: "Bash".into(),
                ok: true,
                detail: "total 0".into()
            }]
        );
        assert!(t.is_empty());
    }

    #[test]
    fn parses_result() {
        let mut t = HashMap::new();
        let ev = parse_line(
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Done.","duration_ms":1500,"num_turns":2,"total_cost_usd":0.01,"session_id":"abc"}"#,
            &mut t,
        );
        match &ev[0] {
            Event::Done { ok, summary, stats } => {
                assert!(ok);
                assert_eq!(summary.as_deref(), Some("Done."));
                assert_eq!(stats.as_deref(), Some("1.5s, 2 turns"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_thinking_delta() {
        let mut tools = Default::default();
        let ev = parse_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me check"}}}"#,
            &mut tools,
        );
        assert_eq!(ev, vec![Event::Thinking("Let me check".into())]);
    }

    #[test]
    fn parses_text_delta() {
        let mut t = HashMap::new();
        let ev = parse_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}}"#,
            &mut t,
        );
        assert_eq!(ev, vec![Event::TextDelta("Hel".into())]);
        let ev = parse_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{"}}}"#,
            &mut t,
        );
        assert_eq!(ev, vec![]);
    }

    #[test]
    fn garbage_is_other() {
        let mut t = HashMap::new();
        assert_eq!(
            parse_line("not json", &mut t),
            vec![Event::Other("not json".into())]
        );
    }
}
