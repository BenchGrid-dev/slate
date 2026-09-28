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
    /// Compositor container id (sway), for close/focus. Not shown to agents.
    #[serde(skip)]
    pub con_id: Option<i64>,
    /// Decoration offsets: container rect minus content rect (title bar, borders).
    #[serde(skip)]
    pub deco: (i32, i32, i32, i32),
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
                con_id: g.map(|g| g.con_id),
                deco: g
                    .map(|g| {
                        (
                            g.content.x - g.rect.x,
                            g.content.y - g.rect.y,
                            g.rect.width - g.content.width,
                            g.rect.height - g.content.height,
                        )
                    })
                    .unwrap_or((0, 0, 0, 0)),
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
            "description": "Press a key or combo on the agent keyboard, e.g. 'Return', 'ctrl+l', 'shift+Tab', 'Escape'. Keys go to the app the agent seat has focused (click a window first). Window-manager shortcuts differ per desktop; use desktop_close / desktop_focus instead of guessing them.",
            "inputSchema": {"type": "object", "properties": {"combo": {"type": "string"}, "seat": {"type": "string", "enum": ["agent", "user"], "description": "agent (default): the agent's own seat, the human keeps their input. user: borrow the human's mouse/keyboard for this action. Needed for GTK4 apps (most GNOME apps), which only listen to the first seat; use it when agent-seat input has no visible effect. Requires approval."}}, "required": ["combo"]}
        },
        {
            "name": "desktop_close",
            "description": "Close a window (id, app_id or title substring) the proper way, as if its close button was clicked. Do not guess keyboard shortcuts for this.",
            "inputSchema": {"type": "object", "properties": {"window": {"type": "string"}}, "required": ["window"]}
        },
        {
            "name": "desktop_focus",
            "description": "Bring a window to the front and give it keyboard focus for the human's seat. Usually unnecessary: desktop_click already focuses the window for the agent's own seat.",
            "inputSchema": {"type": "object", "properties": {"window": {"type": "string"}}, "required": ["window"]}
        },
        {
            "name": "desktop_window_set",
            "description": "Move, resize, maximise or restore a window. Coordinates in screen pixels. Any field may be omitted. fullscreen=true fills the screen; fullscreen=false restores.",
            "inputSchema": {"type": "object", "properties": {
                "window": {"type": "string"},
                "x": {"type": "integer"}, "y": {"type": "integer"},
                "width": {"type": "integer"}, "height": {"type": "integer"},
                "fullscreen": {"type": "boolean"}
            }, "required": ["window"]}
        },
        {
            "name": "desktop_arrange",
            "description": "Lay windows out over the usable screen: side_by_side (left to right), top_bottom, grid, or maximize (one window fills the screen). `windows` is an ordered list of ids, app_ids or title substrings; omit it to arrange all windows on the current workspace.",
            "inputSchema": {"type": "object", "properties": {
                "layout": {"type": "string", "enum": ["side_by_side", "top_bottom", "grid", "maximize"]},
                "windows": {"type": "array", "items": {"type": "string"}}
            }, "required": ["layout"]}
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
                let (ident, crop) = match args.get("window").and_then(Value::as_str) {
                    Some(w) if !w.is_empty() => {
                        let win = resolve(d, w)?;
                        let crop = match (win.width, win.height) {
                            (Some(cw), Some(ch)) if cw > 0 && ch > 0 => {
                                Some((cw as u32, ch as u32))
                            }
                            _ => None,
                        };
                        (Some(win.id), crop)
                    }
                    _ => (None, None),
                };
                let (png, w, h) = d.capture(ident.as_deref(), crop)?;
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
            "desktop_window_set" => {
                let w = args.get("window").and_then(Value::as_str).unwrap_or("");
                let win = resolve(d, w)?;
                let con = win
                    .con_id
                    .ok_or_else(|| anyhow!("no compositor handle for window {}", win.id))?;
                let mut done = vec![];
                if let Some(fs) = args.get("fullscreen").and_then(Value::as_bool) {
                    sway::command_for_con(
                        con,
                        if fs {
                            "fullscreen enable"
                        } else {
                            "fullscreen disable"
                        },
                    )?;
                    done.push(if fs {
                        "fullscreen".to_string()
                    } else {
                        "restored".to_string()
                    });
                }
                let gi = |k: &str| args.get(k).and_then(Value::as_i64);
                let want_w = gi("width").map(|v| v.max(100));
                let want_h = gi("height").map(|v| v.max(100));
                let want_x = gi("x");
                let want_y = gi("y");
                if want_w.is_some() || want_h.is_some() || want_x.is_some() || want_y.is_some() {
                    sway::command_for_con(con, "floating enable")?;
                    // sway acts on the container (title bar, borders) while the agent talks
                    // about content pixels, and the mapping differs between tiled and floating
                    // windows. Apply, measure, and correct once instead of guessing.
                    let mut cur = win.clone();
                    for _ in 0..2 {
                        let (cx, cy) = (cur.x.unwrap_or(0) as i64, cur.y.unwrap_or(0) as i64);
                        let (cw, ch) = (
                            cur.width.unwrap_or(0) as i64,
                            cur.height.unwrap_or(0) as i64,
                        );
                        let ex = want_x.map(|v| v - cx).unwrap_or(0);
                        let ey = want_y.map(|v| v - cy).unwrap_or(0);
                        let ew = want_w.map(|v| v - cw).unwrap_or(0);
                        let eh = want_h.map(|v| v - ch).unwrap_or(0);
                        if ex == 0 && ey == 0 && ew == 0 && eh == 0 {
                            break;
                        }
                        if ew != 0 || eh != 0 {
                            sway::command_for_con(
                                con,
                                &format!(
                                    "resize set {} px {} px",
                                    cw + ew + cur.deco.2 as i64,
                                    ch + eh + cur.deco.3 as i64
                                ),
                            )?;
                        }
                        if ex != 0 || ey != 0 {
                            sway::command_for_con(
                                con,
                                &format!(
                                    "move position {} px {} px",
                                    cx + ex - cur.deco.0 as i64,
                                    cy + ey - cur.deco.1 as i64
                                ),
                            )?;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(80));
                        cur = resolve(d, &win.id)?;
                    }
                    done.push(format!(
                        "now at {},{} {}x{}",
                        cur.x.unwrap_or(0),
                        cur.y.unwrap_or(0),
                        cur.width.unwrap_or(0),
                        cur.height.unwrap_or(0)
                    ));
                }
                text(format!(
                    "{}: {}",
                    win.id,
                    if done.is_empty() {
                        "nothing to change".into()
                    } else {
                        done.join(", ")
                    }
                ))
            }
            "desktop_arrange" => {
                let layout = args
                    .get("layout")
                    .and_then(Value::as_str)
                    .unwrap_or("side_by_side");
                let all = windows(d)?;
                let chosen: Vec<Window> = match args.get("windows").and_then(Value::as_array) {
                    Some(list) if !list.is_empty() => {
                        let mut v = vec![];
                        for r in list.iter().filter_map(Value::as_str) {
                            v.push(resolve(d, r)?);
                        }
                        v
                    }
                    _ => all.into_iter().filter(|w| w.con_id.is_some()).collect(),
                };
                if chosen.is_empty() {
                    anyhow::bail!("no windows to arrange");
                }
                let area = sway::usable_area()?;
                let gap = 8i64;
                let n = chosen.len() as i64;
                let (cols, rows) = match layout {
                    "side_by_side" => (n, 1),
                    "top_bottom" => (1, n),
                    "maximize" => (1, 1),
                    _ => {
                        let c = (n as f64).sqrt().ceil() as i64;
                        (c, (n + c - 1) / c)
                    }
                };
                let cell_w = (area.width as i64 - gap * (cols + 1)) / cols;
                let cell_h = (area.height as i64 - gap * (rows + 1)) / rows;
                let mut placed = vec![];
                for (i, w) in chosen.iter().enumerate() {
                    if layout == "maximize" && i > 0 {
                        break;
                    }
                    let con = w
                        .con_id
                        .ok_or_else(|| anyhow!("no compositor handle for {}", w.id))?;
                    let (col, row) = ((i as i64) % cols, (i as i64) / cols);
                    let x = area.x as i64 + gap + col * (cell_w + gap);
                    let y = area.y as i64 + gap + row * (cell_h + gap);
                    sway::command_for_con(con, "fullscreen disable")?;
                    sway::command_for_con(con, "floating enable")?;
                    sway::command_for_con(con, &format!("resize set {cell_w} px {cell_h} px"))?;
                    sway::command_for_con(con, &format!("move position {x} px {y} px"))?;
                    placed.push(format!("{} -> {x},{y} {cell_w}x{cell_h}", w.app_id));
                }
                text(format!("{layout}: {}", placed.join("; ")))
            }
            "desktop_close" | "desktop_focus" => {
                let w = args.get("window").and_then(Value::as_str).unwrap_or("");
                let win = resolve(d, w)?;
                let con = win.con_id.ok_or_else(|| {
                    anyhow!(
                        "no compositor handle for window {}; this compositor does not expose it",
                        win.id
                    )
                })?;
                let cmd = if name == "desktop_close" {
                    "kill"
                } else {
                    "focus"
                };
                sway::command_for_con(con, cmd)?;
                text(format!(
                    "{} {} ({})",
                    if cmd == "kill" { "closed" } else { "focused" },
                    win.id,
                    win.title
                ))
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
    desktop.settle()?;
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
