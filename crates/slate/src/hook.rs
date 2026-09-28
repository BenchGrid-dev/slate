//! Claude Code hook entry points. Read the hook JSON on stdin, consult slated,
//! print the decision JSON. Never blocks the agent on slated being down:
//! if we cannot reach the daemon we allow and say nothing (exit 0), which
//! leaves Claude Code's own permission handling in charge.

use crate::client::Client;
use anyhow::Result;
use serde_json::{json, Value};
use slate_proto::{Decision, Reply, Request};
use std::io::Read;

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
