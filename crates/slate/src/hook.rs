//! Hook entry points of the agents' programs. Read the hook JSON on stdin, consult
//! slated, print the decision JSON.
//!
//! Claude Code: never blocks the agent on slated being down. If we cannot reach the
//! daemon we allow and say nothing (exit 0), which leaves Claude Code's own
//! permission handling in charge; a call that needs the person's word goes back as
//! "ask", and Claude Code brings it to Slate's permission tool.
//!
//! Codex has neither: a hook's "ask" is not something it acts on, and when it runs
//! for slash nothing of its own asks a person. So its hook asks the person itself and
//! waits for the answer, and without the daemon it refuses. The hook is installed for
//! the whole system (/etc/codex/hooks.json, the one place Codex runs hooks from
//! without a review by the person), so it also runs when someone uses Codex by
//! themselves: no Slate task, and the hook stays out of the way.

use crate::client::Client;
use anyhow::Result;
use serde_json::{json, Value};
use slate_proto::{Decision, Reply, Request};
use std::io::Read;
use std::time::Duration;

fn read_stdin_json() -> Result<Value> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    Ok(serde_json::from_str(&s)?)
}

fn task_id() -> Option<String> {
    std::env::var(slate_proto::ENV_TASK)
        .ok()
        .filter(|s| !s.is_empty())
}

pub fn pre_tool_use() -> Result<()> {
    let input = read_stdin_json()?;
    let tool_name = input
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let tool_input = input.get("tool_input").cloned().unwrap_or(Value::Null);
    let session_id = input
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let cwd = input
        .get("cwd")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from);

    let Ok(mut client) = Client::connect() else {
        return Ok(()); // daemon down: stay out of the way
    };
    let reply = client.call(Request::ToolCheck {
        task_id: task_id(),
        session_id,
        tool_name,
        tool_input,
        cwd,
    })?;
    let (decision, reason) = match reply {
        Reply::ToolChecked {
            decision,
            reason,
            tier,
            ..
        } => (decision, format!("slate: {} ({})", reason, tier.as_str())),
        Reply::Error { message } => {
            slate_proto::log!("slate hook: {message}");
            return Ok(());
        }
        _ => return Ok(()),
    };
    let d = match decision {
        Decision::Allow => "allow",
        Decision::Deny => "deny",
        Decision::Ask => "ask",
    };
    let out = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": d,
            "permissionDecisionReason": reason,
        }
    });
    println!("{out}");
    Ok(())
}

pub fn post_tool_use() -> Result<()> {
    let input = read_stdin_json()?;
    let tool_name = input
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let tool_input = input.get("tool_input").cloned().unwrap_or(Value::Null);
    let session_id = input
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let ok = input.get("tool_error").is_none();
    let Ok(mut client) = Client::connect() else {
        return Ok(());
    };
    let _ = client.call(Request::ToolDone {
        task_id: task_id(),
        session_id,
        tool_name,
        tool_input,
        ok,
    });
    Ok(())
}

/// What Codex is told when a call must not happen: its PreToolUse hook's refusal.
fn codex_deny(reason: &str) {
    let out = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    });
    println!("{out}");
}

/// Codex's PreToolUse hook. Allowing is saying nothing.
pub fn codex_pre_tool_use() -> Result<()> {
    let Some(task_id) = task_id() else {
        return Ok(()); // Codex by itself, not for slash: not Slate's business
    };
    let input = read_stdin_json()?;
    let tool_name = input
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let tool_input = input.get("tool_input").cloned().unwrap_or(Value::Null);
    let session_id = input
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let cwd = input
        .get("cwd")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from);

    let unavailable = "Slate's daemon does not answer, and without it nothing can be approved or undone. Tell the user; do not try another way.";
    let Ok(mut client) = Client::connect() else {
        codex_deny(unavailable);
        return Ok(());
    };
    let checked = client.call(Request::ToolCheck {
        task_id: Some(task_id.clone()),
        session_id: session_id.clone(),
        tool_name: tool_name.clone(),
        tool_input: tool_input.clone(),
        cwd,
    });
    let (decision, reason) = match checked {
        Ok(Reply::ToolChecked {
            decision,
            reason,
            tier,
            ..
        }) => (decision, format!("slate: {} ({})", reason, tier.as_str())),
        Ok(Reply::Error { message }) => {
            codex_deny(&format!("slate: {message}"));
            return Ok(());
        }
        _ => {
            codex_deny(unavailable);
            return Ok(());
        }
    };
    match decision {
        Decision::Allow => {}
        Decision::Deny => codex_deny(&reason),
        Decision::Ask => {
            // The person answers in slash or on the desktop; slated waits for them
            // (ten minutes), and Codex waits for this hook (its timeout is longer).
            client.set_timeout(Some(Duration::from_secs(660)))?;
            match client.call(Request::ApprovalRequest {
                task_id: Some(task_id),
                session_id,
                tool_name,
                tool_input,
            }) {
                Ok(Reply::Approval { allow: true, .. }) => {}
                Ok(Reply::Approval {
                    allow: false,
                    message,
                }) => codex_deny(&format!(
                    "The user did not allow this ({message}). Do not try another way of doing the same thing; say that it was not done."
                )),
                Ok(Reply::Error { message }) => codex_deny(&format!("slate: {message}")),
                _ => codex_deny(unavailable),
            }
        }
    }
    Ok(())
}

/// Codex's PostToolUse hook: the audit log's record that the call ran.
pub fn codex_post_tool_use() -> Result<()> {
    let Some(task_id) = task_id() else {
        return Ok(());
    };
    let input = read_stdin_json()?;
    let tool_name = input
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let tool_input = input.get("tool_input").cloned().unwrap_or(Value::Null);
    let session_id = input
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let Ok(mut client) = Client::connect() else {
        return Ok(());
    };
    let _ = client.call(Request::ToolDone {
        task_id: Some(task_id),
        session_id,
        tool_name,
        tool_input,
        ok: true,
    });
    Ok(())
}

/// The content of /etc/codex/hooks.json that brings Codex's tool calls to Slate: for
/// systems where SlateOS's module has not written it.
pub fn codex_config() -> Result<()> {
    let slate = std::env::current_exe()?;
    let slate = slate.display();
    let hooks = json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "*",
                // Longer than slated waits for the person's answer (ten minutes).
                "hooks": [{"type": "command", "command": format!("{slate} hook codex-pre-tool-use"), "timeout": 900}]
            }],
            "PostToolUse": [{
                "matcher": "*",
                "hooks": [{"type": "command", "command": format!("{slate} hook codex-post-tool-use"), "timeout": 30}]
            }]
        }
    });
    println!("{}", serde_json::to_string_pretty(&hooks)?);
    Ok(())
}
