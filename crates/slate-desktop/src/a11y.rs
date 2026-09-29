//! The accessibility tree (AT-SPI2 over D-Bus): the native way to see and drive
//! applications, before pixels.
//!
//! Every toolkit that supports accessibility (GTK3, GTK4, Qt, Firefox, Chromium,
//! LibreOffice) exposes its widgets on the session's accessibility bus as a tree of
//! objects with a role, a name, states, extents and actions. Reading that tree gives
//! an agent the interactive elements of a window by name; `Action.DoAction` and
//! `EditableText` operate them without a pointer or a keyboard, which also sidesteps
//! toolkits that only listen to the first seat.
//!
//! Applications are matched to compositor windows by pid (sway reports the pid of
//! each view, the bus reports the pid of each connection) and frames by title.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

type Ref = (String, OwnedObjectPath);

const IFACE_ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const IFACE_COMPONENT: &str = "org.a11y.atspi.Component";
const IFACE_ACTION: &str = "org.a11y.atspi.Action";
const IFACE_TEXT: &str = "org.a11y.atspi.Text";
const IFACE_EDITABLE: &str = "org.a11y.atspi.EditableText";
const IFACE_VALUE: &str = "org.a11y.atspi.Value";
const IFACE_CACHE: &str = "org.a11y.atspi.Cache";
const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT_PATH: &str = "/org/a11y/atspi/accessible/root";

/// Never walk more than this many nodes of one application.
const MAX_NODES: usize = 4000;
/// Never return more than this many elements.
const MAX_ELEMENTS: usize = 250;
/// Cap on the text `read` returns.
const MAX_TEXT: usize = 12_000;

/// One node of the tree, as an agent sees it.
#[derive(Debug, Clone, Serialize)]
pub struct Element {
    /// Stable handle for `desktop_element_*` (valid while the daemon runs).
    pub id: String,
    pub role: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Interesting states only: focused, checked, selected, expanded, disabled, editable…
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<String>,
    /// Window-relative extents (same space as window screenshots and `desktop_click`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<i32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
}

#[derive(Debug, Clone)]
struct Node {
    r: Ref,
    parent: Option<Ref>,
    name: String,
    role: u32,
    role_name: Option<String>,
    states: u64,
    interfaces: Vec<String>,
}

pub struct A11y {
    conn: Connection,
    /// element id -> object; the reverse map keeps ids stable across calls.
    handles: HashMap<String, Ref>,
    ids: HashMap<Ref, String>,
    /// element id -> (pid, frame title) the element was read from.
    origins: HashMap<String, (u32, String)>,
    next: u32,
}

impl A11y {
    /// Connect to the session's accessibility bus (asks the session bus for its address).
    pub fn connect() -> Result<Self> {
        let session = Connection::session().context("session bus")?;
        let launcher = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")
            .context("org.a11y.Bus")?;
        let address: String = launcher
            .call("GetAddress", &())
            .context("accessibility bus address (is at-spi2-core running?)")?;
        let conn = zbus::blocking::connection::Builder::address(address.as_str())?
            .build()
            .context("connecting to the accessibility bus")?;
        Ok(Self {
            conn,
            handles: HashMap::new(),
            ids: HashMap::new(),
            origins: HashMap::new(),
            next: 1,
        })
    }

    fn proxy(&self, r: &Ref, iface: &str) -> Result<Proxy<'_>> {
        Ok(Proxy::new(
            &self.conn,
            r.0.clone(),
            r.1.clone(),
            iface.to_string(),
        )?)
    }

    fn id_for(&mut self, r: &Ref) -> String {
        if let Some(id) = self.ids.get(r) {
            return id.clone();
        }
        let id = format!("e{}", self.next);
        self.next += 1;
        self.handles.insert(id.clone(), r.clone());
        self.ids.insert(r.clone(), id.clone());
        id
    }

    /// Where an element was read from: (pid, frame title).
    pub fn origin(&self, id: &str) -> Result<(u32, String)> {
        self.origins
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown element {id}; call desktop_elements first"))
    }

    fn lookup(&self, id: &str) -> Result<Ref> {
        self.handles
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown element {id}; call desktop_elements first"))
    }

    /// Applications on the bus with their pid: (bus name, root object, pid, name).
    pub fn applications(&self) -> Result<Vec<(Ref, u32, String)>> {
        let root: Ref = (REGISTRY.into(), OwnedObjectPath::try_from(ROOT_PATH)?);
        let reg = self.proxy(&root, IFACE_ACCESSIBLE)?;
        let children: Vec<Ref> = reg.call("GetChildren", &())?;
        let dbus = Proxy::new(
            &self.conn,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )?;
        let mut out = vec![];
        for app in children {
            let pid: u32 = dbus
                .call("GetConnectionUnixProcessID", &(app.0.as_str(),))
                .unwrap_or(0);
            let name: String = self
                .proxy(&app, IFACE_ACCESSIBLE)
                .and_then(|p| Ok(p.get_property::<String>("Name")?))
                .unwrap_or_default();
            out.push((app, pid, name));
        }
        Ok(out)
    }

    /// The application whose process is `pid` (or a descendant/ancestor match by name
    /// when the pid is not the one on the bus, e.g. wrappers).
    fn application_for(&self, pid: u32, app_hint: &str) -> Result<Ref> {
        let apps = self.applications()?;
        if let Some((r, _, _)) = apps.iter().find(|(_, p, _)| *p == pid && pid != 0) {
            return Ok(r.clone());
        }
        // Wrappers (nix, flatpak) put a different pid on the bus: try the parent chain.
        let mut cur = pid;
        for _ in 0..6 {
            let Some(parent) = parent_pid(cur) else { break };
            if let Some((r, _, _)) = apps.iter().find(|(_, p, _)| *p == parent) {
                return Ok(r.clone());
            }
            cur = parent;
        }
        // Children of the window's pid (e.g. a launcher that exec'd the real app).
        if let Some((r, _, _)) = apps
            .iter()
            .find(|(_, p, _)| *p != 0 && parent_pid(*p) == Some(pid))
        {
            return Ok(r.clone());
        }
        let hint = app_hint.to_ascii_lowercase();
        if !hint.is_empty() {
            if let Some((r, _, _)) = apps
                .iter()
                .find(|(_, _, n)| n.to_ascii_lowercase().contains(&hint))
            {
                return Ok(r.clone());
            }
        }
        anyhow::bail!(
            "no accessible application for pid {pid} ({app_hint}); the app has no accessibility support, or it started before the accessibility bus"
        )
    }

    /// All nodes of an application, by reference. Cache first, recursion as fallback.
    fn nodes_of(&self, app: &Ref) -> Result<Vec<Node>> {
        if let Ok(nodes) = self.nodes_from_cache(app) {
            if !nodes.is_empty() {
                return Ok(nodes);
            }
        }
        self.nodes_recursive(app)
    }

    fn nodes_from_cache(&self, app: &Ref) -> Result<Vec<Node>> {
        let cache: Ref = (
            app.0.clone(),
            OwnedObjectPath::try_from("/org/a11y/atspi/cache")?,
        );
        let p = self.proxy(&cache, IFACE_CACHE)?;
        // ((so)(so)(so)iiassusau): object, application, parent, index, child count,
        // interfaces, name, role, description, states.
        #[allow(clippy::type_complexity)]
        let items: Vec<(
            Ref,
            Ref,
            Ref,
            i32,
            i32,
            Vec<String>,
            String,
            u32,
            String,
            Vec<u32>,
        )> = p.call("GetItems", &())?;
        Ok(items
            .into_iter()
            .take(MAX_NODES)
            .map(
                |(r, _app, parent, _idx, _child_count, interfaces, name, role, _desc, states)| {
                    Node {
                        r,
                        parent: if parent.1.as_str() == "/org/a11y/atspi/null" {
                            None
                        } else {
                            Some(parent)
                        },
                        name,
                        role,
                        role_name: None,
                        states: state_bits(&states),
                        interfaces,
                    }
                },
            )
            .collect())
    }

    fn nodes_recursive(&self, app: &Ref) -> Result<Vec<Node>> {
        let mut out = vec![];
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((app.clone(), None));
        while let Some((r, parent)) = queue.pop_front() {
            if out.len() >= MAX_NODES {
                break;
            }
            let Ok(acc) = self.proxy(&r, IFACE_ACCESSIBLE) else {
                continue;
            };
            let name: String = acc.get_property("Name").unwrap_or_default();
            let role: u32 = acc.call("GetRole", &()).unwrap_or(0);
            let role_name: String = acc.call("GetRoleName", &()).unwrap_or_default();
            let states: Vec<u32> = acc.call("GetState", &()).unwrap_or_default();
            let children: Vec<Ref> = acc.call("GetChildren", &()).unwrap_or_default();
            let interfaces: Vec<String> = acc.call("GetInterfaces", &()).unwrap_or_default();
            out.push(Node {
                r: r.clone(),
                parent,
                name,
                role,
                role_name: Some(role_name),
                states: state_bits(&states),
                interfaces,
            });
            for c in children {
                queue.push_back((c, Some(r.clone())));
            }
        }
        Ok(out)
    }

    /// The frame (toplevel) of `app` whose title matches, or the active one, or the first.
    fn frame_of<'a>(&self, nodes: &'a [Node], app: &Ref, title: &str) -> Option<&'a Node> {
        let frames: Vec<&Node> = nodes
            .iter()
            .filter(|n| n.parent.as_ref() == Some(app) && is_frame_role(n.role))
            .collect();
        let t = title.trim();
        frames
            .iter()
            .find(|f| !t.is_empty() && f.name.trim() == t)
            .or_else(|| {
                frames.iter().find(|f| {
                    !t.is_empty()
                        && (t.contains(f.name.trim()) || f.name.contains(t))
                        && !f.name.trim().is_empty()
                })
            })
            .or_else(|| frames.iter().find(|f| has_state(f.states, STATE_ACTIVE)))
            .or_else(|| frames.first())
            .copied()
    }

    /// Descendants of `root` (inclusive), in document order as far as the cache tells.
    fn subtree<'a>(&self, nodes: &'a [Node], root: &Ref) -> Vec<&'a Node> {
        let mut children: HashMap<&Ref, Vec<&Node>> = HashMap::new();
        for n in nodes {
            if let Some(p) = &n.parent {
                children.entry(p).or_default().push(n);
            }
        }
        let mut out = vec![];
        let mut stack = vec![root];
        let by_ref: HashMap<&Ref, &Node> = nodes.iter().map(|n| (&n.r, n)).collect();
        while let Some(r) = stack.pop() {
            if let Some(n) = by_ref.get(r) {
                out.push(*n);
            }
            if let Some(cs) = children.get(r) {
                for c in cs.iter().rev() {
                    stack.push(&c.r);
                }
            }
            if out.len() >= MAX_NODES {
                break;
            }
        }
        out
    }

    fn extents(&self, r: &Ref) -> Option<(i32, i32, i32, i32)> {
        let p = self.proxy(r, IFACE_COMPONENT).ok()?;
        // 1 = window coordinates (the toplevel's own space).
        let (x, y, w, h): (i32, i32, i32, i32) = p.call("GetExtents", &(1u32,)).ok()?;
        Some((x, y, w, h))
    }

    fn actions(&self, r: &Ref) -> Vec<String> {
        let Ok(p) = self.proxy(r, IFACE_ACTION) else {
            return vec![];
        };
        let acts: Vec<(String, String, String)> = p.call("GetActions", &()).unwrap_or_default();
        acts.into_iter().map(|(n, _, _)| n).collect()
    }

    fn value_of(&self, n: &Node) -> Option<String> {
        if n.interfaces.iter().any(|i| i == IFACE_VALUE) {
            let p = self.proxy(&n.r, IFACE_VALUE).ok()?;
            let v: OwnedValue = p.get_property("CurrentValue").ok()?;
            let f: f64 = f64::try_from(v).ok()?;
            return Some(trim_float(f));
        }
        if n.interfaces.iter().any(|i| i == IFACE_TEXT) && is_text_field(n.role) {
            let p = self.proxy(&n.r, IFACE_TEXT).ok()?;
            let count: i32 = p.get_property("CharacterCount").ok()?;
            if count <= 0 {
                return None;
            }
            let s: String = p.call("GetText", &(0i32, count.min(200))).ok()?;
            return Some(s);
        }
        None
    }

    fn role_name(&self, n: &Node) -> String {
        if let Some(r) = &n.role_name {
            return r.clone();
        }
        role_name(n.role)
            .map(str::to_string)
            .unwrap_or_else(|| format!("role{}", n.role))
    }

    /// Interactive elements of the window (`pid`, `title`), window-relative extents.
    pub fn elements(
        &mut self,
        pid: u32,
        app_hint: &str,
        title: &str,
        query: Option<&str>,
        all: bool,
    ) -> Result<Vec<Element>> {
        let app = self.application_for(pid, app_hint)?;
        let nodes = self.nodes_of(&app)?;
        let frame = self
            .frame_of(&nodes, &app, title)
            .ok_or_else(|| anyhow!("the application exposes no window on the accessibility bus"))?;
        let frame_ref = frame.r.clone();
        let frame_title = frame.name.clone();
        let sub = self.subtree(&nodes, &frame_ref);
        let q = query.map(|s| s.to_ascii_lowercase());
        let mut out = vec![];
        for n in sub {
            if !has_state(n.states, STATE_SHOWING) || !has_state(n.states, STATE_VISIBLE) {
                continue;
            }
            let interactive = is_interactive_role(n.role)
                || n.interfaces
                    .iter()
                    .any(|i| i == IFACE_ACTION || i == IFACE_EDITABLE)
                || has_state(n.states, STATE_FOCUSABLE);
            if !interactive && !all {
                continue;
            }
            let role = self.role_name(n);
            if let Some(q) = &q {
                if !n.name.to_ascii_lowercase().contains(q) && !role.contains(q.as_str()) {
                    continue;
                }
            }
            let ext = self.extents(&n.r);
            if let Some((_, _, w, h)) = ext {
                if w <= 0 || h <= 0 {
                    continue;
                }
            }
            let id = self.id_for(&n.r);
            self.origins.insert(id.clone(), (pid, frame_title.clone()));
            out.push(Element {
                id,
                role,
                name: n.name.clone(),
                value: self.value_of(n),
                states: interesting_states(n.states),
                x: ext.map(|e| e.0),
                y: ext.map(|e| e.1),
                w: ext.map(|e| e.2),
                h: ext.map(|e| e.3),
                actions: self.actions(&n.r),
            });
            if out.len() >= MAX_ELEMENTS {
                break;
            }
        }
        out.sort_by_key(|e| (e.y.unwrap_or(0) / 8, e.x.unwrap_or(0)));
        Ok(out)
    }

    /// The readable content of the window as an outline: headings, text, links, labels.
    pub fn read(&mut self, pid: u32, app_hint: &str, title: &str) -> Result<String> {
        let app = self.application_for(pid, app_hint)?;
        let nodes = self.nodes_of(&app)?;
        let frame = self
            .frame_of(&nodes, &app, title)
            .ok_or_else(|| anyhow!("the application exposes no window on the accessibility bus"))?;
        let frame_ref = frame.r.clone();
        let frame_title = frame.name.clone();
        let mut out = String::new();
        for n in self.subtree(&nodes, &frame_ref) {
            if !has_state(n.states, STATE_SHOWING) {
                continue;
            }
            let role = self.role_name(n);
            let mut line = String::new();
            let has_text = n.interfaces.iter().any(|i| i == IFACE_TEXT);
            if has_text && !is_container_role(n.role) {
                if let Ok(p) = self.proxy(&n.r, IFACE_TEXT) {
                    let count: i32 = p.get_property("CharacterCount").unwrap_or(0);
                    if count > 0 {
                        let s: String = p
                            .call("GetText", &(0i32, count.min(2000)))
                            .unwrap_or_default();
                        let s = s.trim();
                        // Embedded-object placeholders carry no text.
                        let s: String = s.chars().filter(|c| *c != '\u{fffc}').collect();
                        if !s.trim().is_empty() {
                            line = s.trim().to_string();
                        }
                    }
                }
            }
            if line.is_empty() && !n.name.trim().is_empty() && names_matter(n.role) {
                line = n.name.trim().to_string();
            }
            if line.is_empty() {
                continue;
            }
            let id = self.id_for(&n.r);
            self.origins.insert(id.clone(), (pid, frame_title.clone()));
            out.push_str(&format!("[{role} {id}] {line}\n"));
            if out.len() > MAX_TEXT {
                out.push_str("…(truncated)\n");
                break;
            }
        }
        if out.is_empty() {
            out.push_str("(no readable text; the window may be a canvas or a terminal)\n");
        }
        Ok(out)
    }

    /// Perform an action on the element: the named one, or the first click-like one.
    /// Returns the action performed, or None if the element has no usable action.
    pub fn do_action(&self, id: &str, wanted: Option<&str>) -> Result<Option<String>> {
        let r = self.lookup(id)?;
        let p = self.proxy(&r, IFACE_ACTION)?;
        let acts: Vec<(String, String, String)> = p.call("GetActions", &()).unwrap_or_default();
        let idx = match wanted {
            Some(w) => acts.iter().position(|(n, _, _)| n.eq_ignore_ascii_case(w)),
            None => acts
                .iter()
                .position(|(n, _, _)| {
                    let n = n.to_ascii_lowercase();
                    n == "click" || n == "press" || n == "activate" || n == "jump" || n == "toggle"
                })
                .or(if acts.is_empty() { None } else { Some(0) }),
        };
        let Some(i) = idx else {
            return Ok(None);
        };
        let ok: bool = p.call("DoAction", &(i as i32,))?;
        if !ok {
            anyhow::bail!("the application refused action {:?}", acts[i].0);
        }
        Ok(Some(acts[i].0.clone()))
    }

    /// Element extents in window coordinates, for a pointer fallback.
    pub fn element_extents(&self, id: &str) -> Result<(i32, i32, i32, i32)> {
        let r = self.lookup(id)?;
        self.extents(&r)
            .ok_or_else(|| anyhow!("element {id} has no extents"))
    }

    /// Replace the text of an editable element. Ok(false) when it is not editable.
    pub fn set_text(&self, id: &str, text: &str) -> Result<bool> {
        let r = self.lookup(id)?;
        let Ok(p) = self.proxy(&r, IFACE_EDITABLE) else {
            return Ok(false);
        };
        let ok: bool = match p.call("SetTextContents", &(text,)) {
            Ok(v) => v,
            Err(e) => {
                if e.to_string().contains("UnknownMethod")
                    || e.to_string().contains("UnknownInterface")
                {
                    return Ok(false);
                }
                return Err(e.into());
            }
        };
        Ok(ok)
    }

    pub fn grab_focus(&self, id: &str) -> Result<bool> {
        let r = self.lookup(id)?;
        let p = self.proxy(&r, IFACE_COMPONENT)?;
        Ok(p.call("GrabFocus", &()).unwrap_or(false))
    }

    /// Name and role of a known element, for messages.
    pub fn describe(&self, id: &str) -> String {
        match self.lookup(id) {
            Ok(r) => {
                let name: String = self
                    .proxy(&r, IFACE_ACCESSIBLE)
                    .and_then(|p| Ok(p.get_property::<String>("Name")?))
                    .unwrap_or_default();
                let role: String = self
                    .proxy(&r, IFACE_ACCESSIBLE)
                    .and_then(|p| Ok(p.call::<_, _, String>("GetRoleName", &())?))
                    .unwrap_or_default();
                format!("{role} {name:?}")
            }
            Err(_) => id.to_string(),
        }
    }
}

fn parent_pid(pid: u32) -> Option<u32> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // "pid (comm) state ppid ..." and comm may contain spaces/parens.
    let after = s.rsplit(')').next()?;
    after.split_whitespace().nth(1)?.parse().ok()
}

fn trim_float(f: f64) -> String {
    if f.fract() == 0.0 {
        format!("{}", f as i64)
    } else {
        format!("{f:.2}")
    }
}

// ---- roles and states (AT-SPI2 enums; the numbering is stable since ATK)

const ROLES: &[&str] = &[
    "invalid",
    "accelerator label",
    "alert",
    "animation",
    "arrow",
    "calendar",
    "canvas",
    "check box",
    "check menu item",
    "color chooser",
    "column header",
    "combo box",
    "date editor",
    "desktop icon",
    "desktop frame",
    "dial",
    "dialog",
    "directory pane",
    "drawing area",
    "file chooser",
    "filler",
    "focus traversable",
    "font chooser",
    "frame",
    "glass pane",
    "html container",
    "icon",
    "image",
    "internal frame",
    "label",
    "layered pane",
    "list",
    "list item",
    "menu",
    "menu bar",
    "menu item",
    "option pane",
    "page tab",
    "page tab list",
    "panel",
    "password text",
    "popup menu",
    "progress bar",
    "push button",
    "radio button",
    "radio menu item",
    "root pane",
    "row header",
    "scroll bar",
    "scroll pane",
    "separator",
    "slider",
    "spin button",
    "split pane",
    "status bar",
    "table",
    "table cell",
    "table column header",
    "table row header",
    "tearoff menu item",
    "terminal",
    "text",
    "toggle button",
    "tool bar",
    "tool tip",
    "tree",
    "tree table",
    "unknown",
    "viewport",
    "window",
    "extended",
    "header",
    "footer",
    "paragraph",
    "ruler",
    "application",
    "autocomplete",
    "editbar",
    "embedded",
    "entry",
    "chart",
    "caption",
    "document frame",
    "heading",
    "page",
    "section",
    "redundant object",
    "form",
    "link",
    "input method window",
    "table row",
    "tree item",
    "document spreadsheet",
    "document presentation",
    "document text",
    "document web",
    "document email",
    "comment",
    "list box",
    "grouping",
    "image map",
    "notification",
    "info bar",
    "level bar",
    "title bar",
    "block quote",
    "audio",
    "video",
    "definition",
    "article",
    "landmark",
    "log",
    "marquee",
    "math",
    "rating",
    "timer",
    "static",
    "math fraction",
    "math root",
    "subscript",
    "superscript",
    "description list",
    "description term",
    "description value",
    "footnote",
    "content deletion",
    "content insertion",
    "mark",
    "suggestion",
    "push button menu",
    "switch",
];

fn role_name(role: u32) -> Option<&'static str> {
    ROLES.get(role as usize).copied()
}

fn is_frame_role(role: u32) -> bool {
    matches!(
        role_name(role),
        Some("frame" | "window" | "dialog" | "file chooser" | "alert")
    )
}

fn is_interactive_role(role: u32) -> bool {
    matches!(
        role_name(role),
        Some(
            "check box"
                | "check menu item"
                | "combo box"
                | "entry"
                | "link"
                | "list item"
                | "menu"
                | "menu item"
                | "page tab"
                | "password text"
                | "push button"
                | "push button menu"
                | "radio button"
                | "radio menu item"
                | "scroll bar"
                | "slider"
                | "spin button"
                | "switch"
                | "table cell"
                | "text"
                | "toggle button"
                | "tree item"
                | "tool bar"
                | "icon"
        )
    )
}

fn is_text_field(role: u32) -> bool {
    matches!(
        role_name(role),
        Some("entry" | "text" | "password text" | "spin button" | "combo box")
    )
}

fn is_container_role(role: u32) -> bool {
    matches!(
        role_name(role),
        Some(
            "frame"
                | "window"
                | "dialog"
                | "panel"
                | "filler"
                | "scroll pane"
                | "viewport"
                | "root pane"
                | "layered pane"
                | "split pane"
                | "document frame"
                | "document web"
                | "section"
                | "form"
                | "table"
                | "tree"
                | "list"
                | "tool bar"
                | "menu bar"
                | "page tab list"
                | "grouping"
                | "landmark"
                | "article"
                | "application"
                | "internal frame"
                | "html container"
                | "block quote"
        )
    )
}

fn names_matter(role: u32) -> bool {
    matches!(
        role_name(role),
        Some(
            "label"
                | "heading"
                | "link"
                | "push button"
                | "toggle button"
                | "check box"
                | "radio button"
                | "menu item"
                | "check menu item"
                | "radio menu item"
                | "menu"
                | "page tab"
                | "list item"
                | "tree item"
                | "table cell"
                | "static"
                | "paragraph"
                | "text"
                | "entry"
                | "combo box"
                | "caption"
                | "image"
                | "icon"
                | "status bar"
                | "tool tip"
                | "notification"
                | "info bar"
                | "switch"
                | "push button menu"
        )
    )
}

const STATE_ACTIVE: u32 = 1;
const STATE_CHECKED: u32 = 4;
const STATE_EDITABLE: u32 = 7;
const STATE_ENABLED: u32 = 8;
const STATE_EXPANDABLE: u32 = 9;
const STATE_EXPANDED: u32 = 10;
const STATE_FOCUSABLE: u32 = 11;
const STATE_FOCUSED: u32 = 12;
const STATE_PRESSED: u32 = 20;
const STATE_SELECTED: u32 = 23;
const STATE_SENSITIVE: u32 = 24;
const STATE_SHOWING: u32 = 25;
const STATE_VISIBLE: u32 = 30;
const STATE_HAS_POPUP: u32 = 42;
const STATE_READ_ONLY: u32 = 43;

fn state_bits(words: &[u32]) -> u64 {
    let lo = words.first().copied().unwrap_or(0) as u64;
    let hi = words.get(1).copied().unwrap_or(0) as u64;
    lo | (hi << 32)
}

fn has_state(states: u64, bit: u32) -> bool {
    states & (1u64 << bit) != 0
}

fn interesting_states(states: u64) -> Vec<String> {
    let mut out = vec![];
    if has_state(states, STATE_FOCUSED) {
        out.push("focused".into());
    }
    if has_state(states, STATE_CHECKED) {
        out.push("checked".into());
    }
    if has_state(states, STATE_PRESSED) {
        out.push("pressed".into());
    }
    if has_state(states, STATE_SELECTED) {
        out.push("selected".into());
    }
    if has_state(states, STATE_EXPANDABLE) {
        out.push(
            if has_state(states, STATE_EXPANDED) {
                "expanded"
            } else {
                "collapsed"
            }
            .into(),
        );
    }
    if !has_state(states, STATE_ENABLED) || !has_state(states, STATE_SENSITIVE) {
        out.push("disabled".into());
    }
    if has_state(states, STATE_EDITABLE) {
        out.push("editable".into());
    }
    if has_state(states, STATE_READ_ONLY) {
        out.push("read-only".into());
    }
    if has_state(states, STATE_HAS_POPUP) {
        out.push("has-popup".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_table_matches_the_spec_anchors() {
        assert_eq!(role_name(43), Some("push button"));
        assert_eq!(role_name(61), Some("text"));
        assert_eq!(role_name(79), Some("entry"));
        assert_eq!(role_name(88), Some("link"));
        assert_eq!(role_name(130), Some("switch"));
    }

    #[test]
    fn states_span_two_words() {
        let s = state_bits(&[1 << STATE_SHOWING, 1 << (STATE_HAS_POPUP - 32)]);
        assert!(has_state(s, STATE_SHOWING));
        assert!(has_state(s, STATE_HAS_POPUP));
        assert!(!has_state(s, STATE_FOCUSED));
    }

    #[test]
    fn parent_pid_parses_stat_with_spaces_in_comm() {
        let after = "1234 (my (odd) prog) S 77 1 1".rsplit(')').next().unwrap();
        assert_eq!(after.split_whitespace().nth(1), Some("77"));
    }
}
