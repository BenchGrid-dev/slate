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
    pub identifier: String,
    pub con_id: i64,
    /// Container rectangle, absolute.
    pub rect: Rect,
    /// Content (surface) rectangle, absolute.
    pub content: Rect,
    pub focused: bool,
    pub output: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputGeometry {
    pub name: String,
    pub rect: Rect,
}

fn socket_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SWAYSOCK") {
        return Some(PathBuf::from(p));
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    let rd = std::fs::read_dir(dir).ok()?;
    rd.flatten().map(|e| e.path()).find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("sway-ipc."))
            .unwrap_or(false)
    })
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
pub fn usable_area() -> Result<Rect> {
    let (_, outputs, workspaces) = tree_full()?;
    if let Some(ws) = workspaces.iter().find(|w| w.focused).or(workspaces.first()) {
        return Ok(ws.rect.clone());
    }
    outputs
        .first()
        .map(|o| o.rect.clone())
        .ok_or_else(|| anyhow::anyhow!("no outputs"))
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
