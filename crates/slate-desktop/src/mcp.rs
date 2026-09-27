//! MCP server (stdio, JSON-RPC 2.0) exposing the desktop to agent backends,
//! plus the shared window/launch helpers the CLI uses.

use crate::sway;
use crate::wayland::{Desktop, Seat};
use anyhow::{anyhow, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Clone, serde::Serialize)]
pub struct Window {
    pub id: String,
    pub app_id: String,
    pub title: String,
    pub focused: bool,
    /// Absolute content rectangle, when the compositor tells us.
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

pub fn windows(d: &mut Desktop) -> Result<Vec<Window>> {
    let tls = d.toplevels()?;
    let geo = if sway::available() {
        sway::tree().ok()
    } else {
        None
    };
    Ok(tls
        .into_iter()
        .map(|t| {
            let g = geo
                .as_ref()
                .and_then(|(ws, _)| ws.iter().find(|w| w.identifier == t.identifier));
            Window {
                id: t.identifier,
                app_id: t.app_id,
                title: t.title,
                focused: g.map(|g| g.focused).unwrap_or(false),
                x: g.map(|g| g.content.x),
                y: g.map(|g| g.content.y),
                width: g.map(|g| g.content.width),
                height: g.map(|g| g.content.height),
            }
        })
        .collect())
}

/// Resolve a window reference: identifier, app_id, or title substring.
fn resolve(d: &mut Desktop, reference: &str) -> Result<Window> {
    let wins = windows(d)?;
    let r = reference.to_ascii_lowercase();
    wins.iter()
        .find(|w| w.id == reference)
        .or_else(|| wins.iter().find(|w| w.app_id.to_ascii_lowercase() == r))
        .or_else(|| {
            wins.iter()
                .find(|w| w.title.to_ascii_lowercase().contains(&r))
        })
        .cloned()
        .ok_or_else(|| {
            anyhow!("no window matches {reference:?}; call desktop_windows to list them")
        })
}

pub fn launch(cmd: &str, args: &[String]) -> Result<u32> {
    let child = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(child.id())
}

pub fn probe() -> Result<()> {
    let mut d = Desktop::connect()?;
    println!("transient seat: global {}", d.seat_name.unwrap_or(0));
    for o in d.outputs() {
        println!(
            "output {} {}x{} scale {}",
            o.name, o.width, o.height, o.scale
        );
    }
    println!(
        "sway ipc: {}",
        if sway::available() {
            "yes"
        } else {
            "no (no window geometry)"
        }
    );
    for w in windows(&mut d)? {
        println!(
            "window {} app_id={:?} title={:?} at {:?},{:?} {:?}x{:?}",
            w.id, w.app_id, w.title, w.x, w.y, w.width, w.height
        );
    }
    Ok(())
}

fn tools() -> Value {
    json!([
        {
            "name": "desktop_windows",
            "description": "List open windows on the user's desktop: id, app_id, title, focus, and content position/size in screen pixels.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "desktop_screenshot",
            "description": "Screenshot one window (by id, app_id or title substring) or the whole screen if no window is given. Window screenshots are in window-content coordinates: (0,0) is the window's top-left; use desktop_click with the same window to click what you see.",
            "inputSchema": {"type": "object", "properties": {"window": {"type": "string"}}}
        },
        {
            "name": "desktop_click",
            "description": "Click with the agent's own pointer (the user's mouse is untouched). Coordinates are relative to the window's content if `window` is given, else absolute screen pixels. If clicks have no visible effect in a GTK4/GNOME app, retry with seat='user'.",
            "inputSchema": {"type": "object", "properties": {
                "x": {"type": "number"}, "y": {"type": "number"},
                "window": {"type": "string"},
                "button": {"type": "string", "enum": ["left", "right", "middle"]},
                "count": {"type": "integer", "description": "1 for click, 2 for double-click"},
                "seat": {"type": "string", "enum": ["agent", "user"], "description": "agent (default): the agent's own seat, the human keeps their input. user: borrow the human's mouse/keyboard for this action. Needed for GTK4 apps (most GNOME apps), which only listen to the first seat; use it when agent-seat input has no visible effect. Requires approval."}
            }, "required": ["x", "y"]}
        },
        {
            "name": "desktop_move",
            "description": "Move the agent's pointer without clicking (hover). Same coordinate rules as desktop_click.",
            "inputSchema": {"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}, "window": {"type": "string"}, "seat": {"type": "string", "enum": ["agent", "user"], "description": "agent (default): the agent's own seat, the human keeps their input. user: borrow the human's mouse/keyboard for this action. Needed for GTK4 apps (most GNOME apps), which only listen to the first seat; use it when agent-seat input has no visible effect. Requires approval."}}, "required": ["x", "y"]}
        },
        {
            "name": "desktop_scroll",
            "description": "Scroll at a position. dy > 0 scrolls down, dx > 0 scrolls right (values in pixels, ~15 per notch).",
            "inputSchema": {"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}, "window": {"type": "string"}, "dx": {"type": "number"}, "dy": {"type": "number"}, "seat": {"type": "string", "enum": ["agent", "user"], "description": "agent (default): the agent's own seat, the human keeps their input. user: borrow the human's mouse/keyboard for this action. Needed for GTK4 apps (most GNOME apps), which only listen to the first seat; use it when agent-seat input has no visible effect. Requires approval."}}, "required": ["x", "y"]}
        },
        {
            "name": "desktop_type",
            "description": "Type text with the agent's own keyboard into whatever the agent seat has focused (click a window first). Newlines press Return.",
            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}, "seat": {"type": "string", "enum": ["agent", "user"], "description": "agent (default): the agent's own seat, the human keeps their input. user: borrow the human's mouse/keyboard for this action. Needed for GTK4 apps (most GNOME apps), which only listen to the first seat; use it when agent-seat input has no visible effect. Requires approval."}}, "required": ["text"]}
        },
        {
            "name": "desktop_key",
            "description": "Press a key or combo on the agent keyboard, e.g. 'Return', 'ctrl+l', 'alt+Tab', 'shift+Tab', 'Escape'.",
            "inputSchema": {"type": "object", "properties": {"combo": {"type": "string"}, "seat": {"type": "string", "enum": ["agent", "user"], "description": "agent (default): the agent's own seat, the human keeps their input. user: borrow the human's mouse/keyboard for this action. Needed for GTK4 apps (most GNOME apps), which only listen to the first seat; use it when agent-seat input has no visible effect. Requires approval."}}, "required": ["combo"]}
        },
        {
            "name": "desktop_launch",
            "description": "Start a program on the user's desktop (e.g. 'foot', 'firefox'). Returns the pid. Use desktop_windows afterwards to find its window.",
            "inputSchema": {"type": "object", "properties": {"command": {"type": "string"}, "args": {"type": "array", "items": {"type": "string"}}}, "required": ["command"]}
        }
    ])
}

fn text(s: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": s.into()}]})
}

fn error(s: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": s.into()}], "isError": true})
}

fn point(d: &mut Desktop, args: &Value) -> Result<(f64, f64)> {
    let x = args.get("x").and_then(Value::as_f64).unwrap_or(0.0);
    let y = args.get("y").and_then(Value::as_f64).unwrap_or(0.0);
    match args.get("window").and_then(Value::as_str) {
        Some(w) if !w.is_empty() => {
            let win = resolve(d, w)?;
            let (Some(wx), Some(wy)) = (win.x, win.y) else {
                anyhow::bail!(
                    "window {} has no known position on this compositor; use absolute coordinates",
                    win.id
                );
            };
            Ok((wx as f64 + x, wy as f64 + y))
        }
        _ => Ok((x, y)),
    }
}

fn seat_of(args: &Value) -> Seat {
    Seat::parse(args.get("seat").and_then(Value::as_str).unwrap_or("agent"))
}

fn call(d: &mut Desktop, name: &str, args: &Value) -> Value {
    let seat = seat_of(args);
    let via = if seat == Seat::User {
        " via the user's seat"
    } else {
        ""
    };
    let r: Result<Value> = (|| {
        Ok(match name {
            "desktop_windows" => text(serde_json::to_string_pretty(&windows(d)?)?),
            "desktop_screenshot" => {
                let ident = match args.get("window").and_then(Value::as_str) {
                    Some(w) if !w.is_empty() => Some(resolve(d, w)?.id),
                    _ => None,
                };
                let (png, w, h) = d.capture(ident.as_deref())?;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
                json!({"content": [
                    {"type": "text", "text": format!("{w}x{h} px{}", ident.map(|i| format!(", window {i}")).unwrap_or_else(|| ", full screen".into()))},
                    {"type": "image", "data": b64, "mimeType": "image/png"}
                ]})
            }
            "desktop_click" => {
                let (x, y) = point(d, args)?;
                let button = args.get("button").and_then(Value::as_str).unwrap_or("left");
                let count = args.get("count").and_then(Value::as_u64).unwrap_or(1) as u32;
                d.click(seat, x, y, button, count)?;
                text(format!("clicked {button} at {x:.0},{y:.0}{via}"))
            }
            "desktop_move" => {
                let (x, y) = point(d, args)?;
                d.pointer_move(seat, x, y)?;
                text(format!("pointer at {x:.0},{y:.0}{via}"))
            }
            "desktop_scroll" => {
                let (x, y) = point(d, args)?;
                let dx = args.get("dx").and_then(Value::as_f64).unwrap_or(0.0);
                let dy = args.get("dy").and_then(Value::as_f64).unwrap_or(0.0);
                d.scroll(seat, x, y, dx, dy)?;
                text(format!("scrolled dx={dx} dy={dy} at {x:.0},{y:.0}{via}"))
            }
            "desktop_type" => {
                let t = args.get("text").and_then(Value::as_str).unwrap_or("");
                d.type_text(seat, t)?;
                text(format!("typed {} characters{via}", t.chars().count()))
            }
            "desktop_key" => {
                let c = args.get("combo").and_then(Value::as_str).unwrap_or("");
                d.key(seat, c)?;
                text(format!("pressed {c}{via}"))
            }
            "desktop_launch" => {
                let cmd = args.get("command").and_then(Value::as_str).unwrap_or("");
                let a: Vec<String> = args
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|v| {
                        v.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let pid = launch(cmd, &a)?;
                text(format!("started {cmd} (pid {pid})"))
            }
            other => anyhow::bail!("unknown tool {other}"),
        })
    })();
    match r {
        Ok(v) => v,
        Err(e) => error(format!("{e:#}")),
    }
}

pub fn serve() -> Result<()> {
    let mut desktop = Desktop::connect()?;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => Some(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "slate-desktop", "version": slate_proto::VERSION}
            })),
            "notifications/initialized" | "notifications/cancelled" => None,
            "ping" => Some(json!({})),
            "tools/list" => Some(json!({"tools": tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                Some(call(&mut desktop, name, &args))
            }
            _ => {
                if id.is_some() {
                    writeln!(
                        stdout,
                        "{}",
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("unknown method {method}")}})
                    )?;
                    stdout.flush()?;
                }
                continue;
            }
        };
        if let (Some(id), Some(result)) = (id, result) {
            writeln!(
                stdout,
                "{}",
                json!({"jsonrpc": "2.0", "id": id, "result": result})
            )?;
            stdout.flush()?;
        }
    }
    Ok(())
}
