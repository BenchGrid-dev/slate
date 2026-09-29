//! Minimal sway / i3 IPC client: enough to map foreign-toplevel identifiers to
//! on-screen geometry. Compositor-specific; other compositors get no geometry.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WindowGeometry {
    /// The client's pid, when the compositor knows it.
    pub pid: Option<u32>,
    pub identifier: String,
    pub con_id: i64,
    /// Container rectangle, absolute.
    pub rect: Rect,
    /// Content (surface) rectangle, absolute.
    pub content: Rect,
    pub focused: bool,
    pub output: Option<String>,
    /// Title bar drawn by the compositor (relative to the workspace origin); zero height = none.
    pub deco: Rect,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputGeometry {
    pub name: String,
    pub rect: Rect,
}

/// The live sway socket. `SWAYSOCK` first, but only if it answers: a daemon that
/// outlived a session restart inherits the old value. Otherwise the newest socket in
/// the runtime directory that answers.
fn socket_path() -> Option<PathBuf> {
    let alive = |p: &PathBuf| std::os::unix::net::UnixStream::connect(p).is_ok();
    if let Some(p) = std::env::var_os("SWAYSOCK") {
        let p = PathBuf::from(p);
        if alive(&p) {
            return Some(p);
        }
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    let rd = std::fs::read_dir(dir).ok()?;
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("sway-ipc."))
                .unwrap_or(false)
        })
        .filter_map(|p| {
            let t = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
            Some((t, p))
        })
        .collect();
    candidates.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    candidates.into_iter().map(|(_, p)| p).find(|p| alive(p))
}

fn ipc(msg_type: u32, payload: &str) -> Result<Value> {
    let path =
        socket_path().context("no sway IPC socket (SWAYSOCK unset and none in XDG_RUNTIME_DIR)")?;
    let mut s = UnixStream::connect(&path)?;
    let mut msg = b"i3-ipc".to_vec();
    msg.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    msg.extend_from_slice(&msg_type.to_ne_bytes());
    msg.extend_from_slice(payload.as_bytes());
    s.write_all(&msg)?;
    let mut header = [0u8; 14];
    s.read_exact(&mut header)?;
    if &header[..6] != b"i3-ipc" {
        bail!("bad IPC reply header");
    }
    let len = u32::from_ne_bytes(header[6..10].try_into().unwrap()) as usize;
    let mut body = vec![0u8; len];
    s.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

pub fn available() -> bool {
    socket_path().is_some()
}

/// Run an arbitrary sway command.
pub fn run_command(command: &str) -> Result<()> {
    let reply = ipc(0, command)?;
    let ok = reply
        .as_array()
        .map(|a| {
            a.iter()
                .all(|r| r.get("success").and_then(Value::as_bool).unwrap_or(false))
        })
        .unwrap_or(false);
    if !ok {
        bail!("sway rejected `{command}`: {reply}");
    }
    Ok(())
}

/// Run a sway command against a container, e.g. `kill` or `focus`.
pub fn command_for_con(con_id: i64, command: &str) -> Result<()> {
    let reply = ipc(0, &format!("[con_id={con_id}] {command}"))?; // RUN_COMMAND
    let ok = reply
        .as_array()
        .map(|a| {
            a.iter()
                .all(|r| r.get("success").and_then(Value::as_bool).unwrap_or(false))
        })
        .unwrap_or(false);
    if !ok {
        bail!("sway rejected `{command}` for container {con_id}: {reply}");
    }
    Ok(())
}

fn rect(v: &Value) -> Rect {
    let g = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0) as i32;
    Rect {
        x: g("x"),
        y: g("y"),
        width: g("width"),
        height: g("height"),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceGeometry {
    pub name: String,
    /// Usable area: the output minus panels.
    pub rect: Rect,
    pub focused: bool,
    /// Shown on its output right now.
    pub visible: bool,
    pub output: Option<String>,
}

/// All windows with a foreign toplevel identifier, plus outputs.
pub fn tree() -> Result<(Vec<WindowGeometry>, Vec<OutputGeometry>)> {
    let (w, o, _) = tree_full()?;
    Ok((w, o))
}

/// Windows, outputs and workspaces.
pub fn tree_full() -> Result<(
    Vec<WindowGeometry>,
    Vec<OutputGeometry>,
    Vec<WorkspaceGeometry>,
)> {
    let root = ipc(4, "")?; // GET_TREE
    let mut windows = vec![];
    let mut outputs = vec![];
    let mut workspaces = vec![];
    walk(&root, None, &mut windows, &mut outputs, &mut workspaces);
    Ok((windows, outputs, workspaces))
}

/// The usable rectangle of the focused workspace (falls back to the first output).
/// The usable area of the person's screen: the visible workspace of a real output (the
/// focused one first), never the agent's background screen.
pub fn usable_area() -> Result<Rect> {
    let (_, outputs, workspaces) = tree_full()?;
    let real = |w: &&WorkspaceGeometry| {
        w.output
            .as_deref()
            .map(|o| !crate::mcp::is_background_output(o))
            .unwrap_or(true)
    };
    if let Some(ws) = workspaces
        .iter()
        .filter(real)
        .find(|w| w.focused && w.visible)
        .or_else(|| workspaces.iter().filter(real).find(|w| w.visible))
        .or_else(|| workspaces.iter().find(real))
    {
        return Ok(ws.rect.clone());
    }
    outputs
        .iter()
        .find(|o| !crate::mcp::is_background_output(&o.name))
        .or(outputs.first())
        .map(|o| o.rect.clone())
        .ok_or_else(|| anyhow::anyhow!("no outputs"))
}

/// The usable area of the workspace shown on `output` (for windows that live there).
pub fn area_for_output(output: &str) -> Option<Rect> {
    let (_, _, workspaces) = tree_full().ok()?;
    workspaces
        .iter()
        .filter(|w| w.output.as_deref() == Some(output))
        .find(|w| w.visible)
        .or_else(|| {
            workspaces
                .iter()
                .find(|w| w.output.as_deref() == Some(output))
        })
        .map(|w| w.rect.clone())
}

fn walk(
    node: &Value,
    output: Option<&str>,
    windows: &mut Vec<WindowGeometry>,
    outputs: &mut Vec<OutputGeometry>,
    workspaces: &mut Vec<WorkspaceGeometry>,
) {
    let ty = node.get("type").and_then(Value::as_str).unwrap_or("");
    let name = node.get("name").and_then(Value::as_str);
    let mut current_output = output;
    if ty == "workspace" {
        if let Some(n) = name {
            if !n.starts_with("__") {
                workspaces.push(WorkspaceGeometry {
                    name: n.to_string(),
                    rect: node.get("rect").map(rect).unwrap_or(Rect {
                        x: 0,
                        y: 0,
                        width: 0,
                        height: 0,
                    }),
                    focused: node
                        .get("focused")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        || node
                            .get("focus")
                            .and_then(Value::as_array)
                            .map(|a| !a.is_empty())
                            .unwrap_or(false)
                            && node
                                .get("visible")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                    visible: node
                        .get("visible")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    output: output.map(str::to_string),
                });
            }
        }
    }
    if ty == "output" {
        if let Some(n) = name {
            if !n.starts_with("__") {
                outputs.push(OutputGeometry {
                    name: n.to_string(),
                    rect: node.get("rect").map(rect).unwrap_or(Rect {
                        x: 0,
                        y: 0,
                        width: 0,
                        height: 0,
                    }),
                });
                current_output = name;
            }
        }
    }
    if let Some(id) = node
        .get("foreign_toplevel_identifier")
        .and_then(Value::as_str)
    {
        let r = node.get("rect").map(rect).unwrap_or(Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        });
        let wr = node.get("window_rect").map(rect).unwrap_or(Rect {
            x: 0,
            y: 0,
            width: r.width,
            height: r.height,
        });
        windows.push(WindowGeometry {
            pid: node.get("pid").and_then(Value::as_u64).map(|p| p as u32),
            identifier: id.to_string(),
            con_id: node.get("id").and_then(Value::as_i64).unwrap_or(0),
            content: Rect {
                x: r.x + wr.x,
                y: r.y + wr.y,
                width: wr.width,
                height: wr.height,
            },
            rect: r,
            focused: node
                .get("focused")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            output: current_output.map(str::to_string),
            deco: node.get("deco_rect").map(rect).unwrap_or(Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            }),
        });
    }
    for key in ["nodes", "floating_nodes"] {
        if let Some(children) = node.get(key).and_then(Value::as_array) {
            for c in children {
                walk(c, current_output, windows, outputs, workspaces);
            }
        }
    }
}

/// Keep the real outputs where a desktop without a background screen would have them
/// (in a row from (0,0), their order preserved) and the headless output far away, so the
/// cursor cannot reach it. sway places unpositioned outputs after positioned ones, at
/// their y, which is why every output gets an explicit position. Returns the name of
/// the first real output.
pub fn fix_layout() -> Result<Option<String>> {
    let outs = ipc(3, "")?; // GET_OUTPUTS
    let Some(arr) = outs.as_array() else {
        return Ok(None);
    };
    let mut real: Vec<(String, i64, i64, i64)> = vec![];
    let mut headless: Vec<(String, i64, i64)> = vec![];
    for o in arr {
        let name = o
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let active = o.get("active").and_then(Value::as_bool).unwrap_or(true);
        if name.is_empty() || !active {
            continue;
        }
        let r = o.get("rect").cloned().unwrap_or_default();
        let (x, y, w) = (
            r.get("x").and_then(Value::as_i64).unwrap_or(0),
            r.get("y").and_then(Value::as_i64).unwrap_or(0),
            r.get("width").and_then(Value::as_i64).unwrap_or(0),
        );
        if crate::mcp::is_background_output(&name) {
            headless.push((name, x, y));
        } else {
            real.push((name, x, y, w));
        }
    }
    if headless.is_empty() {
        return Ok(real.first().map(|r| r.0.clone()));
    }
    real.sort_by_key(|r| (r.2, r.1));
    let min_y = real.iter().map(|r| r.2).min().unwrap_or(0);
    let mut x = 0;
    let mut cmds = vec![];
    for (name, ox, oy, w) in &real {
        if *ox != x || *oy != min_y || min_y != 0 {
            cmds.push(format!("output {name} position {x} 0"));
        }
        x += w;
    }
    for (name, hx, hy) in &headless {
        if *hx != 0 || *hy != 30000 {
            cmds.push(format!("output {name} position 0 30000"));
        }
    }
    for c in cmds {
        run_command(&c)?;
    }
    Ok(real.first().map(|r| r.0.clone()))
}

/// The container each seat has focused, by seat name.
pub fn seat_focus() -> Result<std::collections::HashMap<String, i64>> {
    let seats = ipc(101, "")?; // GET_SEATS (sway extension)
    let mut out = std::collections::HashMap::new();
    if let Some(arr) = seats.as_array() {
        for s in arr {
            if let (Some(n), Some(f)) = (
                s.get("name").and_then(Value::as_str),
                s.get("focus").and_then(Value::as_i64),
            ) {
                out.insert(n.to_string(), f);
            }
        }
    }
    Ok(out)
}
