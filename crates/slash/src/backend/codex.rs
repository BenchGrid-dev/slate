//! Codex adapter.
//!
//! Launches `codex exec --json` and parses the JSONL event stream
//! (thread.started, turn.*, item.*). Sessions continue with `codex exec resume <id>`.
//! Codex has no system-prompt flag in exec mode, so session context is prepended
//! to the prompt in a delimited block.

use super::{pump_lines, Backend, Event, TurnRequest};
use crate::config::CodexConfig;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::io::{BufReader, Read};
use std::process::{Command, Stdio};

pub struct Codex {
    cfg: CodexConfig,
    thread_id: Option<String>,
}

impl Codex {
    pub fn new(cfg: CodexConfig) -> Self {
        Self {
            cfg,
            thread_id: None,
        }
    }

    fn command(&self, req: &TurnRequest<'_>) -> Command {
        let mut cmd = Command::new(&self.cfg.bin);
        cmd.arg("exec")
            .arg("--json")
            .arg("--skip-git-repo-check")
            .arg("--sandbox")
            .arg(&self.cfg.sandbox)
            .arg("-C")
            .arg(req.cwd);
        if let Some(m) = &self.cfg.model {
            cmd.arg("-m").arg(m);
        }
        cmd.args(&self.cfg.extra_args);
        if let Some(id) = &self.thread_id {
            cmd.arg("resume").arg(id);
        }
        // Session context already travels inside the prompt (see slash's agent_turn);
        // Codex has no system-prompt flag, so give it the static instructions on the
        // first turn only.
        let prompt = if self.thread_id.is_none() {
            format!("{}\n\n{}", req.context, req.prompt)
        } else {
            req.prompt.to_string()
        };
        cmd.arg(prompt);
        if let Some(t) = req.task_id {
            cmd.env(slate_proto::ENV_TASK, t);
        }
        if let Some(d) = req.desktop_bin {
            // Codex reads MCP servers from config; pass overrides on the command line.
            // It starts MCP servers with a minimal environment, so the display variables
            // must be passed explicitly, and non-interactive runs auto-reject MCP calls
            // unless the server is marked as pre-approved.
            cmd.arg("-c")
                .arg(format!("mcp_servers.desktop.command=\"{}\"", d.display()));
            cmd.arg("-c").arg("mcp_servers.desktop.args=[\"serve\"]");
            cmd.arg("-c")
                .arg("mcp_servers.desktop.default_tools_approval_mode=\"approve\"");
            let env_items: Vec<String> = ["WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "SWAYSOCK"]
                .iter()
                .filter_map(|k| {
                    std::env::var(k)
                        .ok()
                        .map(|v| format!("{k}=\"{}\"", v.replace('"', "\\\"")))
                })
                .collect();
            if !env_items.is_empty() {
                cmd.arg("-c").arg(format!(
                    "mcp_servers.desktop.env={{{}}}",
                    env_items.join(",")
                ));
            }
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

impl Backend for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn session_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }

    fn reset(&mut self) {
        self.thread_id = None;
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
            .with_context(|| format!("launching {} (is Codex installed?)", self.cfg.bin))?;

        let stdout = child.stdout.take().context("no stdout")?;
        let mut stderr = child.stderr.take().context("no stderr")?;
        let stderr_thread = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });

        let mut thread_id = self.thread_id.clone();
        let mut saw_done = false;
        pump_lines(BufReader::new(stdout), parse_line, &mut |ev| {
            match &ev {
                Event::SessionStarted(id) => thread_id = Some(id.clone()),
                Event::Done { .. } => saw_done = true,
                _ => {}
            }
            on_event(ev);
        })?;

        let status = child.wait()?;
        let err = stderr_thread.join().unwrap_or_default();
        self.thread_id = thread_id;
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
        if !saw_done {
            on_event(Event::Done {
                ok: true,
                summary: None,
                stats: None,
            });
        }
        Ok(())
    }
}

pub fn parse_line(line: &str) -> Vec<Event> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![Event::Other(line.to_string())];
    };
    let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
    let item = v.get("item");
    let item_type = item
        .and_then(|i| i.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let s = |k: &str| {
        item.and_then(|i| i.get(k))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    match (ty, item_type) {
        ("thread.started", _) => v
            .get("thread_id")
            .and_then(Value::as_str)
            .map(|id| vec![Event::SessionStarted(id.to_string())])
            .unwrap_or_default(),
        ("turn.completed", _) => vec![Event::Done {
            ok: true,
            summary: None,
            stats: v
                .pointer("/usage/output_tokens")
                .and_then(Value::as_u64)
                .map(|n| format!("{n} output tokens")),
        }],
        ("turn.failed", _) => vec![Event::Done {
            ok: false,
            summary: v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string),
            stats: None,
        }],
        ("error", _) => vec![Event::Done {
            ok: false,
            summary: v.get("message").and_then(Value::as_str).map(str::to_string),
            stats: None,
        }],
        ("item.completed", "reasoning") => s("text")
            .map(|t| vec![Event::Thinking(t)])
            .unwrap_or_default(),
        ("item.completed", "agent_message") => s("text")
            .filter(|t| !t.trim().is_empty())
            .map(|t| vec![Event::Text(t)])
            .unwrap_or_default(),
        ("item.started", "command_execution") => vec![Event::ToolStart {
            name: "shell".into(),
            detail: s("command").unwrap_or_default(),
        }],
        ("item.completed", "command_execution") => {
            let code = item
                .and_then(|i| i.get("exit_code"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            vec![Event::ToolEnd {
                name: "shell".into(),
                ok: code == 0,
                detail: s("aggregated_output").unwrap_or_default(),
            }]
        }
        ("item.completed", "file_change") => {
            let files = item
                .and_then(|i| i.get("changes"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.get("path").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            vec![Event::ToolEnd {
                name: "edit".into(),
                ok: true,
                detail: files,
            }]
        }
        ("item.started", "mcp_tool_call") => vec![Event::ToolStart {
            name: format!(
                "{}.{}",
                s("server").unwrap_or_default(),
                s("tool").unwrap_or_default()
            ),
            detail: String::new(),
        }],
        ("item.completed", "mcp_tool_call") => vec![Event::ToolEnd {
            name: format!(
                "{}.{}",
                s("server").unwrap_or_default(),
                s("tool").unwrap_or_default()
            ),
            ok: s("status").as_deref() != Some("failed"),
            detail: String::new(),
        }],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_started() {
        assert_eq!(
            parse_line(r#"{"type":"thread.started","thread_id":"t1"}"#),
            vec![Event::SessionStarted("t1".into())]
        );
    }

    #[test]
    fn agent_message_and_command() {
        assert_eq!(
            parse_line(
                r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"hi"}}"#
            ),
            vec![Event::Text("hi".into())]
        );
        assert_eq!(
            parse_line(
                r#"{"type":"item.started","item":{"id":"i2","type":"command_execution","command":"ls","status":"in_progress"}}"#
            ),
            vec![Event::ToolStart {
                name: "shell".into(),
                detail: "ls".into()
            }]
        );
        assert_eq!(
            parse_line(
                r#"{"type":"item.completed","item":{"id":"i2","type":"command_execution","command":"ls","aggregated_output":"a\n","exit_code":0,"status":"completed"}}"#
            ),
            vec![Event::ToolEnd {
                name: "shell".into(),
                ok: true,
                detail: "a\n".into()
            }]
        );
    }

    #[test]
    fn turn_completed_is_done() {
        match &parse_line(
            r#"{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":5}}"#,
        )[0]
        {
            Event::Done { ok, stats, .. } => {
                assert!(ok);
                assert_eq!(stats.as_deref(), Some("5 output tokens"));
            }
            other => panic!("{other:?}"),
        }
    }
}
