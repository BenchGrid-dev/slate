//! Shared types for the Slate agent runtime.
//!
//! Everything that crosses a process boundary (slash <-> slated <-> slate CLI)
//! is defined here. Transport is JSON lines over a unix socket; see [`socket_path`].
//! Each request carries an `id`; the reply carries the same `id`. A UI client
//! that has sent `ui.attach` also receives unsolicited `event` messages.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Environment variable carrying the current task id into agent subprocesses.
/// Hooks and MCP servers inherit it from the backend, which inherits it from slash.
pub const ENV_TASK: &str = "SLATE_TASK";
/// Environment variable overriding the daemon socket path.
pub const ENV_SOCK: &str = "SLATE_SOCK";

/// How much human involvement an action needs before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Read-only. Runs silently.
    Observe,
    /// Reversible. Runs silently after a snapshot; the user can undo.
    Reversible,
    /// Destructive, sends data off-machine, or touches credentials. Needs explicit approval.
    Confirm,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Observe => "observe",
            Tier::Reversible => "reversible",
            Tier::Confirm => "confirm",
        }
    }
}

/// What the daemon decided about a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Deny,
    /// Needs a human. The backend will consult the permission tool next.
    Ask,
}

/// Which agent backend drives a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Claude,
    Codex,
    Other(String),
}

/// Requests a client sends to slated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Ping,
    /// A UI (slash) begins a task. Returns `TaskStarted`.
    TaskStart {
        backend: Backend,
        prompt: String,
        cwd: PathBuf,
    },
    TaskEnd {
        task_id: String,
        ok: bool,
    },
    /// A hook asks what to do with a tool call before it runs. Returns `ToolChecked`.
    ToolCheck {
        task_id: Option<String>,
        session_id: Option<String>,
        tool_name: String,
        tool_input: serde_json::Value,
        cwd: Option<PathBuf>,
    },
    /// A hook reports a tool call finished. Returns `Ok`.
    ToolDone {
        task_id: Option<String>,
        session_id: Option<String>,
        tool_name: String,
        tool_input: serde_json::Value,
        ok: bool,
    },
    /// The permission tool asks a human. Blocks until answered. Returns `Approval`.
    ApprovalRequest {
        task_id: Option<String>,
        session_id: Option<String>,
        tool_name: String,
        tool_input: serde_json::Value,
    },
    /// A UI attaches to receive `Event::ApprovalNeeded` for a task.
    UiAttach {
        task_id: String,
    },
    /// A UI answers an approval event.
    ApprovalAnswer {
        approval_id: String,
        allow: bool,
        /// Allow the same tool for the rest of the task without asking.
        remember: bool,
    },
    /// Roll back the filesystem to the snapshot taken for a task (default: the last one).
    Undo {
        task_id: Option<String>,
    },
    /// Preview what `Undo` would touch.
    UndoPreview {
        task_id: Option<String>,
    },
    AuditTail {
        n: usize,
    },
    Tasks {
        n: usize,
    },
    /// Store something the user asked to remember.
    MemoryAdd {
        text: String,
        task_id: Option<String>,
    },
    /// Recent memories, newest last. `query` filters by substring when set.
    MemoryList {
        n: usize,
        query: Option<String>,
    },
    MemoryForget {
        memory_id: String,
    },
}

/// A request on the wire: `{"id": "...", "op": "...", ...}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub id: String,
    #[serde(flatten)]
    pub request: Request,
}

/// Replies from slated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Reply {
    Ok,
    Pong {
        version: String,
        snapshots: bool,
    },
    Error {
        message: String,
    },
    TaskStarted {
        task_id: String,
    },
    ToolChecked {
        tier: Tier,
        decision: Decision,
        reason: String,
        /// Set when a snapshot was taken (or already existed) for this task.
        snapshot: Option<String>,
    },
    Approval {
        allow: bool,
        message: String,
    },
    UndoResult {
        task_id: String,
        restored: Vec<PathBuf>,
        deleted: Vec<PathBuf>,
        note: String,
    },
    Audit {
        entries: Vec<AuditEntry>,
    },
    Tasks {
        tasks: Vec<TaskSummary>,
    },
    Memories {
        memories: Vec<Memory>,
    },
    MemoryAdded {
        memory_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub ts: u64,
    pub text: String,
    pub task_id: Option<String>,
}

/// A reply on the wire: `{"id": "...", "result": "...", ...}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyEnvelope {
    pub id: String,
    #[serde(flatten)]
    pub reply: Reply,
}

/// Unsolicited messages to an attached UI: `{"event": "...", ...}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    ApprovalNeeded {
        approval_id: String,
        tool_name: String,
        summary: String,
        tier: Tier,
        reason: String,
    },
    /// The approval was resolved elsewhere (timeout, another UI, task ended).
    ApprovalResolved { approval_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Unix milliseconds.
    pub ts: u64,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub kind: AuditKind,
    pub tool_name: Option<String>,
    pub summary: String,
    pub tier: Option<Tier>,
    pub decision: Option<Decision>,
    pub snapshot: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    TaskStart,
    TaskEnd,
    ToolCheck,
    ToolDone,
    Approval,
    Snapshot,
    Undo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub task_id: String,
    pub started: u64,
    pub ended: Option<u64>,
    pub backend: Backend,
    pub prompt: String,
    pub cwd: PathBuf,
    pub snapshot: Option<String>,
    pub tool_calls: u32,
}

/// Where the daemon listens. `$SLATE_SOCK`, else `$XDG_RUNTIME_DIR/slate/slated.sock`,
/// else `/tmp/slate-<uid>/slated.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os(ENV_SOCK) {
        return PathBuf::from(p);
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("slate").join("slated.sock");
    }
    let uid = unsafe { libc_getuid() };
    PathBuf::from(format!("/tmp/slate-{uid}")).join("slated.sock")
}

extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

/// Per-user state directory: `$XDG_STATE_HOME/slate` or `~/.local/state/slate`.
pub fn state_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(p).join("slate");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".local").join("state").join("slate")
}

/// One-line human summary of a tool call, shared by every UI.
pub fn summarize_tool(tool_name: &str, input: &serde_json::Value) -> String {
    let get = |k: &str| input.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let s = match tool_name {
        "Bash" => get("command").unwrap_or_default(),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            get("file_path").unwrap_or_default()
        }
        "Glob" | "Grep" => get("pattern").unwrap_or_default(),
        "WebFetch" => get("url").unwrap_or_default(),
        "WebSearch" => get("query").unwrap_or_default(),
        "Agent" | "Task" => get("description").unwrap_or_default(),
        _ => input.to_string(),
    };
    let first = s.lines().next().unwrap_or("");
    if first.chars().count() > 120 {
        let cut: String = first.chars().take(119).collect();
        format!("{cut}…")
    } else {
        first.to_string()
    }
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trips() {
        let e = Envelope {
            id: "1".into(),
            request: Request::ToolCheck {
                task_id: Some("t".into()),
                session_id: None,
                tool_name: "Bash".into(),
                tool_input: serde_json::json!({"command": "ls"}),
                cwd: None,
            },
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"op\":\"tool_check\""));
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, "1");
    }

    #[test]
    fn reply_round_trips() {
        let r = ReplyEnvelope {
            id: "1".into(),
            reply: Reply::ToolChecked {
                tier: Tier::Confirm,
                decision: Decision::Ask,
                reason: "x".into(),
                snapshot: None,
            },
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"result\":\"tool_checked\""));
        assert!(s.contains("\"tier\":\"confirm\""));
    }

    #[test]
    fn request_envelopes_have_no_duplicate_keys() {
        for request in [
            Request::MemoryForget {
                memory_id: "m".into(),
            },
            Request::TaskEnd {
                task_id: "t".into(),
                ok: true,
            },
            Request::ApprovalAnswer {
                approval_id: "a".into(),
                allow: true,
                remember: false,
            },
            Request::Undo { task_id: None },
        ] {
            let s = serde_json::to_string(&Envelope {
                id: "7".into(),
                request,
            })
            .unwrap();
            assert_eq!(s.matches("\"id\":").count(), 1, "{s}");
            let back: Envelope = serde_json::from_str(&s).unwrap();
            assert_eq!(back.id, "7");
        }
    }

    #[test]
    fn reply_envelopes_have_no_duplicate_keys() {
        for reply in [
            Reply::MemoryAdded {
                memory_id: "m".into(),
            },
            Reply::TaskStarted {
                task_id: "t".into(),
            },
            Reply::Pong {
                version: "v".into(),
                snapshots: false,
            },
        ] {
            let s = serde_json::to_string(&ReplyEnvelope {
                id: "7".into(),
                reply,
            })
            .unwrap();
            assert_eq!(s.matches("\"id\":").count(), 1, "{s}");
            let back: ReplyEnvelope = serde_json::from_str(&s).unwrap();
            assert_eq!(back.id, "7");
        }
    }

    #[test]
    fn summaries() {
        assert_eq!(
            summarize_tool("Bash", &serde_json::json!({"command": "ls -la\nmore"})),
            "ls -la"
        );
        assert_eq!(
            summarize_tool("Edit", &serde_json::json!({"file_path": "/a/b"})),
            "/a/b"
        );
    }
}
