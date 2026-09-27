//! A minimal MCP server over stdio exposing one tool, `approve`, for use as
//! Claude Code's `--permission-prompt-tool mcp__slate__approve`.
//!
//! JSON-RPC 2.0, newline-delimited. Only the methods Claude Code needs.

use crate::client::Client;
use anyhow::Result;
use serde_json::{json, Value};
use slate_proto::{Reply, Request};
use std::io::{BufRead, Write};
use std::time::Duration;

const PROTOCOL_VERSION: &str = "2024-11-05";

pub fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let response = match method {
            "initialize" => Some(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "slate", "version": slate_proto::VERSION}
            })),
            "notifications/initialized" | "notifications/cancelled" => None,
            "ping" => Some(json!({})),
            "tools/list" => Some(json!({"tools": [{
                "name": "approve",
                "description": "Slate approval broker. Asks the human at the Slate UI whether a tool call may run.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tool_name": {"type": "string"},
                        "tool_input": {"type": "object"},
                        "input": {"type": "object"},
                        "tool_use_id": {"type": "string"}
                    }
                }
            }]})),
            "tools/call" => Some(tools_call(&params)),
            _ => {
                if id.is_some() {
                    let err = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("unknown method {method}")}});
                    writeln!(stdout, "{err}")?;
                    stdout.flush()?;
                }
                continue;
            }
        };
        if let (Some(id), Some(result)) = (id, response) {
            let out = json!({"jsonrpc": "2.0", "id": id, "result": result});
            writeln!(stdout, "{out}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn tools_call(params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    if name != "approve" {
        return json!({"content": [{"type": "text", "text": format!("unknown tool {name}")}], "isError": true});
    }
    let tool_name = args
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // Claude Code has used both `input` and `tool_input` for the payload; accept either.
    let tool_input = args
        .get("tool_input")
        .or_else(|| args.get("input"))
        .cloned()
        .unwrap_or(Value::Null);
    let task_id = std::env::var(slate_proto::ENV_TASK)
        .ok()
        .filter(|s| !s.is_empty());

    let decision = (|| -> Result<Value> {
        let mut client = Client::connect()?;
        client.set_timeout(Some(Duration::from_secs(660)))?;
        let reply = client.call(Request::ApprovalRequest {
            task_id,
            session_id: None,
            tool_name: tool_name.clone(),
            tool_input: tool_input.clone(),
        })?;
        Ok(match reply {
            Reply::Approval { allow: true, .. } => {
                json!({"behavior": "allow", "updatedInput": tool_input})
            }
            Reply::Approval {
                allow: false,
                message,
            } => json!({"behavior": "deny", "message": message}),
            Reply::Error { message } => {
                json!({"behavior": "deny", "message": format!("slate: {message}")})
            }
            _ => json!({"behavior": "deny", "message": "slate: unexpected reply"}),
        })
    })()
    .unwrap_or_else(
        |e| json!({"behavior": "deny", "message": format!("slate unavailable: {e:#}")}),
    );

    json!({"content": [{"type": "text", "text": decision.to_string()}]})
}
