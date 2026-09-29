//! MCP server (stdio, JSON-RPC 2.0) exposing the desktop to agent backends,
//! plus the shared window/launch helpers the CLI uses.

use crate::a11y::A11y;
use crate::sway;
use crate::wayland::{Desktop, Seat};
use anyhow::{anyhow, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Borrowing the human's seat ("controlling"): visible in the panel, cancellable
/// with Esc through the compositor's `controlling` mode.
#[derive(Debug, Default)]
pub struct Takeover {
    pub active_until: Option<std::time::Instant>,
    pub cancelled_until: Option<std::time::Instant>,
}

pub static TAKEOVER: std::sync::Mutex<Takeover> = std::sync::Mutex::new(Takeover {
    active_until: None,
    cancelled_until: None,
});

static A11Y: std::sync::Mutex<Option<A11y>> = std::sync::Mutex::new(None);

/// Run `f` with the accessibility connection, connecting on first use.
fn with_a11y<T>(f: impl FnOnce(&mut A11y) -> Result<T>) -> Result<T> {
    let mut guard = A11Y.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(A11y::connect()?);
    }
    let r = f(guard.as_mut().expect("connected"));
    if r.is_err() {
        // A dead bus connection would poison every later call: reconnect next time.
        if let Err(e) = &r {
            if e.to_string().contains("Connection") || e.to_string().contains("disconnected") {
                *guard = None;
            }
        }
    }
    r
}

const TAKEOVER_LINGER: std::time::Duration = std::time::Duration::from_secs(6);
const TAKEOVER_CANCEL_HOLD: std::time::Duration = std::time::Duration::from_secs(60);

fn sway_mode(mode: &str) {
    let _ = sway::run_command(&format!("mode {mode}"));
}

/// Called before a user-seat action. Errors if the user cancelled recently.
fn takeover_begin() -> Result<()> {
    let mut t = TAKEOVER.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    if t.cancelled_until.map(|u| u > now).unwrap_or(false) {
        anyhow::bail!("the user cancelled control of their mouse and keyboard (Esc); do not retry seat=user until they ask");
    }
    let was_active = t.active_until.map(|u| u > now).unwrap_or(false);
    t.active_until = Some(now + TAKEOVER_LINGER);
    if !was_active {
        sway_mode("controlling");
    }
    Ok(())
}

/// Periodic housekeeping: leave the `controlling` mode when the takeover lingers out.
pub fn takeover_tick() {
    let mut t = TAKEOVER.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    if let Some(u) = t.active_until {
        if u <= now {
            t.active_until = None;
            sway_mode("default");
        }
    }
}

pub fn takeover_cancel() {
    let mut t = TAKEOVER.lock().unwrap_or_else(|e| e.into_inner());
    t.active_until = None;
    t.cancelled_until = Some(std::time::Instant::now() + TAKEOVER_CANCEL_HOLD);
    sway_mode("default");
}

pub fn takeover_active() -> bool {
    let t = TAKEOVER.lock().unwrap_or_else(|e| e.into_inner());
    t.active_until
        .map(|u| u > std::time::Instant::now())
        .unwrap_or(false)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Window {
    pub id: String,
    /// Client pid (from the compositor), for the accessibility bus. Not shown to agents.
    #[serde(skip)]
    pub pid: Option<u32>,
    /// Compositor title bar (screen pixels), if any. Not shown to agents.
    #[serde(skip)]
    pub titlebar: Option<(i32, i32, i32, i32)>,
    /// Compositor container id (sway), for close/focus. Not shown to agents.
    #[serde(skip)]
    pub con_id: Option<i64>,
    /// Decoration offsets: container rect minus content rect (title bar, borders).
    #[serde(skip)]
    pub deco: (i32, i32, i32, i32),
    pub app_id: String,
    pub title: String,
    pub focused: bool,
    /// "screen" (on the user's display) or "background" (on the agent's own, unseen output).
    pub location: String,
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
                titlebar: g.and_then(|g| {
                    if g.deco.height > 0 {
                        let area = g
                            .output
                            .as_deref()
                            .and_then(sway::area_for_output)
                            .or_else(|| sway::usable_area().ok())?;
                        Some((
                            g.deco.x + area.x,
                            g.deco.y + area.y,
                            g.deco.width,
                            g.deco.height,
                        ))
                    } else {
                        None
                    }
                }),
                con_id: g.map(|g| g.con_id),
                pid: g.and_then(|g| g.pid),
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
                location: if g
                    .and_then(|g| g.output.as_deref())
                    .map(is_background_output)
                    .unwrap_or(false)
                {
                    "background".into()
                } else {
                    "screen".into()
                },
                x: g.map(|g| g.content.x),
                y: g.map(|g| g.content.y),
                width: g.map(|g| g.content.width),
                height: g.map(|g| g.content.height),
            }
        })
        .collect())
}

/// The agent's own screen: a headless output the person never sees. Windows there are
/// "in the background"; `workspace agent` is assigned to it by the SlateOS sway profile.
pub const BACKGROUND_WORKSPACE: &str = "agent";

/// Pids `desktop_launch` was asked to open on the user's screen (`where: "here"`),
/// with when: the window watcher leaves those alone.
static HERE_PIDS: std::sync::Mutex<Vec<(u32, std::time::Instant)>> =
    std::sync::Mutex::new(Vec::new());

fn allow_on_screen(pid: u32) {
    let mut v = HERE_PIDS.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|(_, t)| t.elapsed() < std::time::Duration::from_secs(120));
    v.push((pid, std::time::Instant::now()));
}

fn allowed_on_screen(pid: u32) -> bool {
    let v = HERE_PIDS.lock().unwrap_or_else(|e| e.into_inner());
    v.iter()
        .any(|(p, t)| *p == pid && t.elapsed() < std::time::Duration::from_secs(120))
}

/// Was this process started by the agent's engine (directly or through any number
/// of shells)? slash marks the engine's environment, which everything it starts
/// inherits; the process tree is the fallback.
pub fn spawned_by_agent(pid: u32) -> bool {
    if let Ok(env) = std::fs::read(format!("/proc/{pid}/environ")) {
        let key = format!("{}=", slate_proto::ENV_TASK);
        if env
            .split(|b| *b == 0)
            .any(|kv| kv.starts_with(key.as_bytes()))
        {
            return true;
        }
    }
    let mut p = pid;
    for _ in 0..64 {
        let Ok(status) = std::fs::read_to_string(format!("/proc/{p}/status")) else {
            break;
        };
        let mut name = "";
        let mut ppid = 0u32;
        for line in status.lines() {
            if let Some(v) = line.strip_prefix("Name:") {
                name = v.trim();
            } else if let Some(v) = line.strip_prefix("PPid:") {
                ppid = v.trim().parse().unwrap_or(0);
            }
        }
        let base = name.trim_start_matches('.').trim_end_matches("-wrapped");
        if matches!(base, "claude" | "codex") {
            return true;
        }
        if ppid <= 1 {
            break;
        }
        p = ppid;
    }
    false
}

/// A window just appeared (the daemon's window watcher). If the agent's engine
/// started its process and nobody asked for it on the user's screen, it goes to the
/// background screen: the agent works there unless the user wants to watch.
pub fn place_new_window(con_id: i64, pid: Option<u32>, app_id: &str) {
    let Some(pid) = pid else { return };
    if background_output().is_none() || allowed_on_screen(pid) || !spawned_by_agent(pid) {
        return;
    }
    if let Ok((wins, _)) = sway::tree() {
        if let Some(w) = wins.iter().find(|w| w.con_id == con_id) {
            if w.output
                .as_deref()
                .map(is_background_output)
                .unwrap_or(false)
            {
                return;
            }
        }
    }
    match sway::command_for_con(
        con_id,
        &format!("move container to workspace {BACKGROUND_WORKSPACE}"),
    ) {
        Ok(()) => slate_proto::log!(
            "slate-desktop: {app_id} (pid {pid}) was started by the agent: moved to the background screen"
        ),
        Err(e) => slate_proto::log!("slate-desktop: placing {app_id} (pid {pid}): {e:#}"),
    }
}

pub fn is_background_output(name: &str) -> bool {
    name.starts_with("HEADLESS-")
}

/// The background output, if the compositor has one.
fn background_output() -> Option<String> {
    let (_, outs) = sway::tree().ok()?;
    outs.into_iter()
        .map(|o| o.name)
        .find(|n| is_background_output(n))
}

/// The user's output: the first one that is not the background.
fn screen_output() -> Option<String> {
    let (_, outs) = sway::tree().ok()?;
    outs.into_iter()
        .map(|o| o.name)
        .find(|n| !is_background_output(n))
}

fn background_count(d: &mut Desktop) -> usize {
    windows(d)
        .map(|ws| ws.iter().filter(|w| w.location == "background").count())
        .unwrap_or(0)
}

/// The compositor window an element belongs to (by the pid the tree was read from).
fn window_of_element(d: &mut Desktop, id: &str) -> Result<Window> {
    let (pid, title) = with_a11y(|a| a.origin(id))?;
    let wins = windows(d)?;
    wins.iter()
        .find(|w| w.pid == Some(pid) && w.title == title)
        .or_else(|| wins.iter().find(|w| w.pid == Some(pid)))
        .cloned()
        .ok_or_else(|| anyhow!("the window of element {id} is gone"))
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
            "description": "List open windows: id, app_id, title, focus, location (\"screen\" = visible to the user, \"background\" = on the agent's own unseen screen), and content position/size in layout pixels.",
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
                "verify": {"type": "boolean", "description": "default true: return a screenshot of the window after the click so you can see the effect"},
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
            "description": "Type text into a window. Always pass `window`: the tool focuses it for the seat in use and verifies before typing, and the result says which window received the text. Newlines press Return.",
            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}, "window": {"type": "string", "description": "target window (id, app_id or title substring); strongly recommended"}, "verify": {"type": "boolean", "description": "default true: return a screenshot of the last clicked window afterwards"}, "seat": {"type": "string", "enum": ["agent", "user"], "description": "agent (default): the agent's own seat, the human keeps their input. user: borrow the human's mouse/keyboard for this action. Needed for GTK4 apps (most GNOME apps), which only listen to the first seat; use it when agent-seat input has no visible effect. Requires approval."}}, "required": ["text"]}
        },
        {
            "name": "desktop_key",
            "description": "Press a key or combo on the agent keyboard, e.g. 'Return', 'ctrl+l', 'shift+Tab', 'Escape'. Keys go to the app the agent seat has focused (click a window first). Window-manager shortcuts differ per desktop; use desktop_close / desktop_focus instead of guessing them.",
            "inputSchema": {"type": "object", "properties": {"combo": {"type": "string"}, "window": {"type": "string", "description": "target window (id, app_id or title substring); strongly recommended"}, "verify": {"type": "boolean", "description": "default true: return a screenshot of the last clicked window afterwards"}, "seat": {"type": "string", "enum": ["agent", "user"], "description": "agent (default): the agent's own seat, the human keeps their input. user: borrow the human's mouse/keyboard for this action. Needed for GTK4 apps (most GNOME apps), which only listen to the first seat; use it when agent-seat input has no visible effect. Requires approval."}}, "required": ["combo"]}
        },
        {
            "name": "desktop_close",
            "description": "Close a window (id, app_id or title substring) the proper way, as if its close button was clicked. Do not guess keyboard shortcuts for this.",
            "inputSchema": {"type": "object", "properties": {"window": {"type": "string"}}, "required": ["window"]}
        },
        {
            "name": "desktop_focus",
            "description": "Bring a window to the front on the user's screen (this also gives it the user's keyboard focus, so use it only when the user asked to see or use that window). For a background window use desktop_show.",
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
            "name": "desktop_status",
            "description": "Whether the agent is currently controlling the user's own mouse/keyboard (seat=\"user\" actions). Panels poll this.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "desktop_seats",
            "description": "Which window the agent's seat and the user's seat currently have focused. Use it when unsure where typing would go.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "desktop_launch",
            "description": "Start a program and wait for its window. `where` = \"background\" (default): the window opens on the agent's own screen, which the user never sees, so you can work without disturbing them; \"here\": on the user's screen, for tasks the user wants to watch or that involve what they are doing. Returns the window id and where it is.",
            "inputSchema": {"type": "object", "properties": {"command": {"type": "string"}, "args": {"type": "array", "items": {"type": "string"}}, "where": {"type": "string", "enum": ["background", "here"]}}, "required": ["command"]}
        },
        {
            "name": "desktop_show",
            "description": "Bring a background window (or all of them, when no window is given) to the user's screen, e.g. when the user says \"show me\" or a result should be looked at.",
            "inputSchema": {"type": "object", "properties": {"window": {"type": "string"}}}
        },
        {
            "name": "desktop_hide",
            "description": "Move a window (or every non-terminal window on the user's screen, when no window is given) to the agent's background screen, out of the user's way.",
            "inputSchema": {"type": "object", "properties": {"window": {"type": "string"}}}
        },
        {
            "name": "desktop_elements",
            "description": "The interactive elements of a window from its accessibility tree: id, role, name, value, states, window-relative extents (x,y,w,h in the same space as window screenshots and desktop_click), actions. Prefer this over screenshots: act on elements by id with desktop_element_click / desktop_element_set_text and read text with desktop_read. `query` filters by name or role; `all` includes non-interactive nodes.",
            "inputSchema": {"type": "object", "properties": {
                "window": {"type": "string", "description": "window id, app_id or title substring"},
                "query": {"type": "string"},
                "all": {"type": "boolean"}
            }, "required": ["window"]}
        },
        {
            "name": "desktop_read",
            "description": "The readable text of a window from its accessibility tree (headings, paragraphs, labels, links, field contents), one line per node tagged with role and element id. Use instead of reading screenshots.",
            "inputSchema": {"type": "object", "properties": {"window": {"type": "string"}}, "required": ["window"]}
        },
        {
            "name": "desktop_element_click",
            "description": "Activate an element from desktop_elements by id: through its accessibility action when it has one (no pointer needed; works in every toolkit including GTK4), otherwise a pointer click at its centre through the chosen seat. `action` selects a specific action name.",
            "inputSchema": {"type": "object", "properties": {
                "id": {"type": "string"},
                "action": {"type": "string"},
                "seat": {"type": "string", "enum": ["agent", "user"]},
                "verify": {"type": "boolean"}
            }, "required": ["id"]}
        },
        {
            "name": "desktop_element_set_text",
            "description": "Replace the text of an editable element (entry, text field, document) by id through the accessibility EditableText interface; falls back to focusing the element and typing through the chosen seat. Says which path was used.",
            "inputSchema": {"type": "object", "properties": {
                "id": {"type": "string"},
                "text": {"type": "string"},
                "seat": {"type": "string", "enum": ["agent", "user"]},
                "verify": {"type": "boolean"}
            }, "required": ["id", "text"]}
        },
    ])
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
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

/// The window a seat currently has focused, if the compositor tells us.
fn focused_window(d: &mut Desktop, seat: Seat) -> Option<Window> {
    let name = d.seat_display_name(seat)?;
    let con = *sway::seat_focus().ok()?.get(&name)?;
    windows(d).ok()?.into_iter().find(|w| w.con_id == Some(con))
}

/// A point on `win` where a click reaches it: on its title bar (or the top strip of
/// its content when the app draws its own), away from anything stacked above it.
/// Background windows pile up in the middle of the agent's screen, so a covered
/// window there is first brought to the top of the pile (its size and place kept).
fn focus_point(win: &Window) -> Result<(f64, f64)> {
    let bar: (f64, f64, f64, f64) = match win.titlebar {
        Some((tx, ty, tw, th)) => (tx as f64, ty as f64, tw as f64, th as f64),
        None => (
            win.x.unwrap_or(0) as f64,
            win.y.unwrap_or(0) as f64,
            win.width.unwrap_or(200) as f64,
            12.0,
        ),
    };
    let centre = (bar.0 + bar.2 / 2.0, bar.1 + bar.3 / 2.0);
    let Some(con) = win.con_id else {
        return Ok(centre);
    };
    let Ok((geos, _)) = sway::tree() else {
        return Ok(centre);
    };
    let Some(me) = geos.iter().find(|g| g.con_id == con) else {
        return Ok(centre);
    };
    // Title bars are drawn above the container rectangle, relative to the workspace.
    let ws = sway::area_for_output(me.output.as_deref().unwrap_or("")).unwrap_or(sway::Rect {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    });
    let above: Vec<&sway::WindowGeometry> = geos
        .iter()
        .filter(|g| g.z > me.z && g.workspace == me.workspace && g.con_id != con)
        .collect();
    let inside = |x: f64, y: f64, r: &sway::Rect| {
        x >= r.x as f64
            && x < (r.x + r.width) as f64
            && y >= r.y as f64
            && y < (r.y + r.height) as f64
    };
    let covered = |x: f64, y: f64| {
        above.iter().any(|g| {
            let bar = sway::Rect {
                x: ws.x + g.deco.x,
                y: ws.y + g.deco.y,
                width: g.deco.width,
                height: g.deco.height,
            };
            inside(x, y, &g.rect) || (g.deco.height > 0 && inside(x, y, &bar))
        })
    };
    let y = centre.1;
    if !covered(centre.0, y) {
        return Ok(centre);
    }
    let mut x = bar.0 + 16.0;
    while x < bar.0 + bar.2 - 8.0 {
        if !covered(x, y) {
            return Ok((x, y));
        }
        x += 40.0;
    }
    if win.location != "background" {
        anyhow::bail!(
            "{} ({}) is covered by other windows; click inside it where it is visible, or move the windows above it",
            win.id,
            win.title
        );
    }
    // Re-adding a floating container puts it on top of the pile; size and position
    // are restored in the same command so nothing else changes. `move position`
    // places the title bar's top-left corner (workspace-relative), which is what
    // deco_rect gives; without a title bar it is the container itself.
    let (px, py) = if me.deco.height > 0 {
        (me.deco.x, me.deco.y)
    } else {
        (me.rect.x - ws.x, me.rect.y - ws.y)
    };
    sway::command_for_con(
        con,
        &format!(
            "floating disable, floating enable, resize set {} px {} px, move position {} px {} px",
            me.content.width, me.content.height, px, py
        ),
    )?;
    std::thread::sleep(std::time::Duration::from_millis(150));
    // Click where the window is now, not where it was.
    if let Ok((geos, _)) = sway::tree() {
        if let Some(g) = geos.iter().find(|g| g.con_id == con) {
            if g.deco.height > 0 {
                return Ok((
                    ws.x as f64 + g.deco.x as f64 + g.deco.width as f64 / 2.0,
                    ws.y as f64 + g.deco.y as f64 + g.deco.height as f64 / 2.0,
                ));
            }
            return Ok((
                g.content.x as f64 + g.content.width as f64 / 2.0,
                g.content.y as f64 + 6.0,
            ));
        }
    }
    Ok(centre)
}

/// Make sure `seat` will type into `win`: focus it, then verify. For the agent seat
/// this is a click on the compositor's title bar (or the top edge of the content
/// when the app draws its own), for the user's seat a compositor focus command.
fn ensure_focus(d: &mut Desktop, seat: Seat, win: &Window) -> Result<()> {
    if focused_window(d, seat)
        .map(|w| w.id == win.id)
        .unwrap_or(false)
    {
        return Ok(());
    }
    match seat {
        Seat::Agent => {
            let (x, y) = focus_point(win)?;
            d.click(seat, x, y, "left", 1)?;
        }
        Seat::User => {
            if win.location == "background" {
                anyhow::bail!(
                    "{} ({}) is in the background, where the user's seat cannot go. Use the agent seat, or desktop_show it first.",
                    win.id, win.title
                );
            }
            let con = win
                .con_id
                .ok_or_else(|| anyhow!("no compositor handle for {}", win.id))?;
            sway::command_for_con(con, "focus")?;
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    match focused_window(d, seat) {
        Some(w) if w.id == win.id => Ok(()),
        Some(w) => {
            anyhow::bail!(
            "could not focus {} ({}); the {} seat is on {} ({}). Click inside the target first.",
            win.id, win.title, if seat == Seat::User { "user's" } else { "agent" }, w.id, w.title
        )
        }
        None => anyhow::bail!(
            "could not determine the seat's focus; click inside the target window first"
        ),
    }
}

fn seat_of(args: &Value) -> Seat {
    Seat::parse(args.get("seat").and_then(Value::as_str).unwrap_or("agent"))
}

/// A screenshot of `window` (or the whole screen) as MCP image content, best effort.
fn verify_image(d: &mut Desktop, window: Option<&str>) -> Vec<Value> {
    std::thread::sleep(std::time::Duration::from_millis(350));
    let (ident, crop) = match window {
        Some(w) => match resolve(d, w) {
            Ok(win) => {
                let crop = match (win.width, win.height) {
                    (Some(cw), Some(ch)) if cw > 0 && ch > 0 => Some((cw as u32, ch as u32)),
                    _ => None,
                };
                (Some(win.id), crop)
            }
            Err(_) => return vec![json!({"type": "text", "text": "(window is gone)"})],
        },
        None => (None, None),
    };
    match d.capture(ident.as_deref(), crop) {
        Ok((png, w, h)) => vec![
            json!({"type": "text", "text": format!("after: {w}x{h} px{}", ident.map(|i| format!(", window {i}")).unwrap_or_default())}),
            json!({"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png"}),
        ],
        Err(e) => vec![json!({"type": "text", "text": format!("(could not capture: {e:#})")})],
    }
}

fn with_verify(d: &mut Desktop, mut result: Value, window: Option<&str>, args: &Value) -> Value {
    if args.get("verify").and_then(Value::as_bool) == Some(false) {
        return result;
    }
    let extra = verify_image(d, window);
    if let Some(arr) = result.get_mut("content").and_then(Value::as_array_mut) {
        arr.extend(extra);
    }
    result
}

pub fn call(d: &mut Desktop, name: &str, args: &Value) -> Value {
    // Status and cancel never touch the seat.
    match name {
        "desktop_status" => {
            let bg = background_count(d);
            return json!({"content": [{"type": "text", "text": json!({"controlling": takeover_active(), "background_windows": bg, "background_screen": background_output().is_some()}).to_string()}]});
        }
        "desktop_takeover_cancel" => {
            takeover_cancel();
            return text("takeover cancelled: user-seat input is refused for a minute");
        }
        _ => {}
    }
    if seat_of(args) == Seat::User
        && matches!(
            name,
            "desktop_click" | "desktop_type" | "desktop_key" | "desktop_scroll" | "desktop_move"
        )
    {
        if let Err(e) = takeover_begin() {
            return error(format!("{e:#}"));
        }
    }
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
                let win = args
                    .get("window")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(w) = &win {
                    d.last_window = Some(w.clone());
                }
                let r = text(format!("clicked {button} at {x:.0},{y:.0}{via}. Check the screenshot below before claiming the click worked."));
                with_verify(
                    d,
                    r,
                    win.as_deref().or(d.last_window.clone().as_deref()),
                    args,
                )
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
            "desktop_type" | "desktop_key" => {
                // Where will this go? Focus the requested window first, and always say
                // which window received the input.
                let target = match args.get("window").and_then(Value::as_str) {
                    Some(w) if !w.is_empty() => Some(resolve(d, w)?),
                    _ => None,
                };
                if let Some(win) = &target {
                    ensure_focus(d, seat, win)?;
                    d.last_window = Some(win.id.clone());
                }
                let into = focused_window(d, seat)
                    .map(|w| format!(" into {} ({})", w.app_id, w.title))
                    .unwrap_or_else(|| " (focus unknown: click the target window first)".into());
                let r = if name == "desktop_type" {
                    let t = args.get("text").and_then(Value::as_str).unwrap_or("");
                    d.type_text(seat, t)?;
                    text(format!("typed {} characters{via}{into}", t.chars().count()))
                } else {
                    let c = args.get("combo").and_then(Value::as_str).unwrap_or("");
                    d.key(seat, c)?;
                    text(format!("pressed {c}{via}{into}"))
                };
                let lw = target.map(|w| w.id).or_else(|| d.last_window.clone());
                with_verify(d, r, lw.as_deref(), args)
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
                    // The command values are an affine function of the content geometry with
                    // an offset we do not know exactly (title bar, borders), so start from the
                    // decoration guess, then shift the *commanded* values by the measured error.
                    let mut cur = win.clone();
                    let (cx0, cy0) = (cur.x.unwrap_or(0) as i64, cur.y.unwrap_or(0) as i64);
                    let (cw0, ch0) = (
                        cur.width.unwrap_or(0) as i64,
                        cur.height.unwrap_or(0) as i64,
                    );
                    let mut cmd_w = want_w.unwrap_or(cw0) + cur.deco.2 as i64;
                    let mut cmd_h = want_h.unwrap_or(ch0) + cur.deco.3 as i64;
                    let area = sway::usable_area().unwrap_or(sway::Rect {
                        x: 0,
                        y: 0,
                        width: 0,
                        height: 0,
                    });
                    let mut cmd_x = want_x.unwrap_or(cx0) - cur.deco.0 as i64 - area.x as i64;
                    let mut cmd_y = want_y.unwrap_or(cy0) - cur.deco.1 as i64 - area.y as i64;
                    for attempt in 0..3 {
                        if attempt > 0 {
                            let (cx, cy) = (cur.x.unwrap_or(0) as i64, cur.y.unwrap_or(0) as i64);
                            let (cw, ch) = (
                                cur.width.unwrap_or(0) as i64,
                                cur.height.unwrap_or(0) as i64,
                            );
                            let ex = want_x.map(|v| v - cx).unwrap_or(0);
                            let ey = want_y.map(|v| v - cy).unwrap_or(0);
                            let ew = want_w.map(|v| v - cw).unwrap_or(0);
                            let eh = want_h.map(|v| v - ch).unwrap_or(0);
                            if ex.abs() <= 1 && ey.abs() <= 1 && ew.abs() <= 1 && eh.abs() <= 1 {
                                break;
                            }
                            cmd_x += ex;
                            cmd_y += ey;
                            cmd_w += ew;
                            cmd_h += eh;
                        }
                        if want_w.is_some() || want_h.is_some() {
                            sway::command_for_con(
                                con,
                                &format!("resize set {cmd_w} px {cmd_h} px"),
                            )?;
                        }
                        if want_x.is_some() || want_y.is_some() {
                            sway::command_for_con(
                                con,
                                &format!("move position {cmd_x} px {cmd_y} px"),
                            )?;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(120));
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
                    // Cell origin in screen pixels; sway's floating `move position` is
                    // relative to the workspace origin, so subtract it when commanding.
                    let x = area.x as i64 + gap + col * (cell_w + gap);
                    let y = area.y as i64 + gap + row * (cell_h + gap);
                    let (mx, my) = (x - area.x as i64, y - area.y as i64);
                    sway::command_for_con(con, "fullscreen disable")?;
                    sway::command_for_con(con, "floating enable")?;
                    sway::command_for_con(con, &format!("resize set {cell_w} px {cell_h} px"))?;
                    sway::command_for_con(con, &format!("move position {mx} px {my} px"))?;
                    placed.push(format!("{} -> {x},{y} {cell_w}x{cell_h}", w.app_id));
                }
                text(format!("{layout}: {}", placed.join("; ")))
            }
            "desktop_seats" => {
                let mut lines = vec![];
                for (label, seat) in [("agent", Seat::Agent), ("user", Seat::User)] {
                    let name = d.seat_display_name(seat).unwrap_or_else(|| "?".into());
                    let focus = focused_window(d, seat)
                        .map(|w| format!("{} ({}) [{}]", w.app_id, w.title, w.id))
                        .unwrap_or_else(|| "nothing / unknown".into());
                    lines.push(format!("{label} seat {name}: {focus}"));
                }
                text(lines.join("\n"))
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
                if name == "desktop_close" {
                    sway::command_for_con(con, "kill")?;
                    return Ok(text(format!("closed {} ({})", win.id, win.title)));
                }
                // Focusing a background window would drag the user's focus onto the
                // background screen; bring the window to them instead.
                if win.location == "background" {
                    let screen = screen_output().ok_or_else(|| anyhow!("no output"))?;
                    sway::command_for_con(
                        con,
                        &format!("move container to output {screen}, focus"),
                    )?;
                    return Ok(text(format!(
                        "{} ({}) was in the background: brought to the user's screen and focused",
                        win.id, win.title
                    )));
                }
                sway::command_for_con(con, "focus")?;
                text(format!("focused {} ({})", win.id, win.title))
            }
            "desktop_elements" => {
                let w = args.get("window").and_then(Value::as_str).unwrap_or("");
                let win = resolve(d, w)?;
                let pid = win
                    .pid
                    .ok_or_else(|| anyhow!("no pid for window {} on this compositor", win.id))?;
                let query = args
                    .get("query")
                    .and_then(Value::as_str)
                    .filter(|q| !q.is_empty());
                let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
                let (elements, total) = with_a11y(|a| {
                    let e = a.elements(pid, &win.app_id, &win.title, query, all)?;
                    Ok((e, a.last_total()))
                })?;
                d.last_window = Some(win.id.clone());
                let mut lines = vec![format!(
                    "{} elements (of {} nodes) in {} ({}) — coordinates are window-relative",
                    elements.len(),
                    total,
                    win.id,
                    win.title
                )];
                for e in &elements {
                    let mut l = format!("{}  {}", e.id, e.role);
                    if !e.name.is_empty() {
                        l.push_str(&format!("  {:?}", e.name));
                    }
                    if let Some(v) = &e.value {
                        l.push_str(&format!("  = {:?}", truncate(v, 80)));
                    }
                    if !e.states.is_empty() {
                        l.push_str(&format!("  [{}]", e.states.join(",")));
                    }
                    if let (Some(x), Some(y), Some(w2), Some(h)) = (e.x, e.y, e.w, e.h) {
                        l.push_str(&format!("  @{x},{y} {w2}x{h}"));
                    }
                    if !e.actions.is_empty() {
                        l.push_str(&format!("  actions: {}", e.actions.join("/")));
                    }
                    lines.push(l);
                }
                text(lines.join("\n"))
            }
            "desktop_read" => {
                let w = args.get("window").and_then(Value::as_str).unwrap_or("");
                let win = resolve(d, w)?;
                let pid = win
                    .pid
                    .ok_or_else(|| anyhow!("no pid for window {} on this compositor", win.id))?;
                let body = with_a11y(|a| a.read(pid, &win.app_id, &win.title))?;
                d.last_window = Some(win.id.clone());
                text(format!("{} ({}):\n{}", win.id, win.title, body))
            }
            "desktop_element_click" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let wanted = args.get("action").and_then(Value::as_str);
                let what = with_a11y(|a| Ok(a.describe(id)))?;
                let done = with_a11y(|a| a.do_action(id, wanted))?;
                let r = match done {
                    Some(action) => text(format!("{action} on {what} through the accessibility tree. Check the screenshot below.")),
                    None => {
                        // No action: a pointer click at the element's centre, in its window.
                        let (ex, ey, ew, eh) = with_a11y(|a| a.element_extents(id))?;
                        let win = window_of_element(d, id)?;
                        if seat == Seat::User { takeover_begin()?; }
                        let (x, y) = (win.x.unwrap_or(0) as f64 + ex as f64 + ew as f64 / 2.0, win.y.unwrap_or(0) as f64 + ey as f64 + eh as f64 / 2.0);
                        d.click(seat, x, y, "left", 1)?;
                        d.last_window = Some(win.id.clone());
                        text(format!("{what} has no accessibility action; clicked its centre at {x:.0},{y:.0}{via}. Check the screenshot below."))
                    }
                };
                let lw = d.last_window.clone();
                with_verify(d, r, lw.as_deref(), args)
            }
            "desktop_element_set_text" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let t = args.get("text").and_then(Value::as_str).unwrap_or("");
                let what = with_a11y(|a| Ok(a.describe(id)))?;
                let r = if with_a11y(|a| a.set_text(id, t))? {
                    text(format!(
                        "set the text of {what} through the accessibility tree ({} characters).",
                        t.chars().count()
                    ))
                } else {
                    // Not editable over the bus: focus it and type.
                    let win = window_of_element(d, id)?;
                    ensure_focus(d, seat, &win)?;
                    if !with_a11y(|a| a.grab_focus(id))? {
                        let (ex, ey, ew, eh) = with_a11y(|a| a.element_extents(id))?;
                        if seat == Seat::User {
                            takeover_begin()?;
                        }
                        d.click(
                            seat,
                            win.x.unwrap_or(0) as f64 + ex as f64 + ew as f64 / 2.0,
                            win.y.unwrap_or(0) as f64 + ey as f64 + eh as f64 / 2.0,
                            "left",
                            1,
                        )?;
                    }
                    d.key(seat, "ctrl+a")?;
                    d.type_text(seat, t)?;
                    d.last_window = Some(win.id.clone());
                    text(format!("{what} is not editable over the accessibility bus; focused it and typed {} characters{via}.", t.chars().count()))
                };
                let lw = d.last_window.clone();
                with_verify(d, r, lw.as_deref(), args)
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
                let bg = background_output();
                let wanted = args
                    .get("where")
                    .and_then(Value::as_str)
                    .unwrap_or("background");
                // `assign`, not a for_window move: the window maps straight onto its
                // workspace. A move after mapping would first give it the user's
                // focus for a moment (the window flashes on their screen).
                let target = if wanted == "here" || bg.is_none() {
                    screen_output().map(|o| format!("output {o}"))
                } else {
                    Some(format!("workspace {BACKGROUND_WORKSPACE}"))
                };
                let before: std::collections::HashSet<String> =
                    windows(d)?.into_iter().map(|w| w.id).collect();
                let pid = launch(cmd, &a)?;
                if wanted == "here" {
                    allow_on_screen(pid);
                }
                // A rule keyed on the pid catches the window before it is shown.
                if let Some(t) = &target {
                    let _ = sway::run_command(&format!("assign [pid={pid}] {t}"));
                }
                // Wait for the window (up to 8 s), place it if the rule missed, report it.
                let mut placed: Option<Window> = None;
                for _ in 0..32 {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    if let Ok(ws) = windows(d) {
                        if let Some(w) = ws.into_iter().find(|w| !before.contains(&w.id)) {
                            placed = Some(w);
                            break;
                        }
                    }
                }
                match placed {
                    Some(mut w) => {
                        let want_bg = target.as_deref().map(|t| t.starts_with("workspace")).unwrap_or(false);
                        if want_bg && w.location != "background" {
                            if let Some(con) = w.con_id {
                                let _ = sway::command_for_con(con, &format!("move container to workspace {BACKGROUND_WORKSPACE}"));
                                std::thread::sleep(std::time::Duration::from_millis(200));
                                if let Ok(ws) = windows(d) {
                                    if let Some(n) = ws.into_iter().find(|x| x.id == w.id) { w = n; }
                                }
                            }
                        }
                        d.last_window = Some(w.id.clone());
                        text(format!(
                            "started {cmd} (pid {pid}): window {} ({}) is {}{}",
                            w.id,
                            w.title,
                            if w.location == "background" { "in the background (the user does not see it; desktop_show brings it to their screen)" } else { "on the user's screen" },
                            if bg.is_none() && wanted != "here" { "; this desktop has no background screen" } else { "" }
                        ))
                    }
                    None => text(format!("started {cmd} (pid {pid}); no window appeared within 8 s (a background service, or it handed off to a running instance). Use desktop_windows.")),
                }
            }
            "desktop_show" | "desktop_hide" => {
                let to_bg = name == "desktop_hide";
                let bg = background_output();
                if to_bg && bg.is_none() {
                    anyhow::bail!("this desktop has no background screen (the SlateOS profile creates one at login)");
                }
                let wins = match args.get("window").and_then(Value::as_str) {
                    Some(w) if !w.is_empty() => vec![resolve(d, w)?],
                    _ => windows(d)?
                        .into_iter()
                        .filter(|w| {
                            if to_bg {
                                w.location == "screen" && w.app_id != "foot"
                            } else {
                                w.location == "background"
                            }
                        })
                        .collect(),
                };
                let screen = screen_output().ok_or_else(|| anyhow!("no output"))?;
                let mut moved = vec![];
                for w in &wins {
                    let Some(con) = w.con_id else { continue };
                    let cmd = if to_bg {
                        format!("move container to workspace {BACKGROUND_WORKSPACE}")
                    } else {
                        format!("move container to output {screen}, focus")
                    };
                    if sway::command_for_con(con, &cmd).is_ok() {
                        moved.push(format!("{} ({})", w.id, w.title));
                    }
                }
                text(if moved.is_empty() {
                    if to_bg {
                        "nothing to hide".to_string()
                    } else {
                        "no windows in the background".to_string()
                    }
                } else {
                    format!(
                        "{} {}: {}",
                        if to_bg {
                            "hidden in the background"
                        } else {
                            "brought to the user's screen"
                        },
                        moved.len(),
                        moved.join(", ")
                    )
                })
            }
            other => anyhow::bail!("unknown tool {other}"),
        })
    })();
    match r {
        Ok(v) => v,
        Err(e) => error(format!("{e:#}")),
    }
}

/// Where tool calls go: the session daemon (one long-lived seat) when it is
/// running, else an in-process Desktop with its own seat.
enum Backend {
    Daemon(crate::daemon::Client),
    Local(Box<Desktop>),
}

impl Backend {
    fn open() -> Result<Self> {
        if let Some(c) = crate::daemon::Client::connect() {
            return Ok(Backend::Daemon(c));
        }
        let mut d = Desktop::connect()?;
        d.settle()?;
        Ok(Backend::Local(Box::new(d)))
    }

    fn call(&mut self, name: &str, args: &Value) -> Value {
        match self {
            Backend::Daemon(c) => c
                .call(name, args)
                .unwrap_or_else(|e| error(format!("desktop daemon: {e:#}"))),
            Backend::Local(d) => call(d, name, args),
        }
    }
}

pub fn serve() -> Result<()> {
    let mut backend = Backend::open()?;
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
                Some(backend.call(name, &args))
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

/// One tool call from the CLI, through the daemon when it runs.
pub fn cli_call(name: &str, args: Value) -> Result<Value> {
    let mut b = Backend::open()?;
    if let Backend::Local(_) = b {
        // A fresh seat: give clients a moment to bind it (see Desktop::settle).
    }
    let r = b.call(name, &args);
    if r.get("isError").and_then(Value::as_bool).unwrap_or(false) {
        let msg = r["content"][0]["text"]
            .as_str()
            .unwrap_or("error")
            .to_string();
        anyhow::bail!("{msg}");
    }
    Ok(r)
}
