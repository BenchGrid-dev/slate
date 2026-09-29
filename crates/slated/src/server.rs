//! Unix-socket server: JSON lines, one thread per connection.

use crate::audit::Audit;
use crate::memory::MemoryStore;
use crate::policy;
use crate::snapshot::{SnapshotInfo, Snapshotter};
use crate::tasks::{new_task_id, Task, TaskStore};
use anyhow::{Context, Result};
use slate_proto::{
    now_millis, summarize_tool, AuditEntry, AuditKind, Decision, Envelope, Event, Reply,
    ReplyEnvelope, Request, Tier,
};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);
/// Snapshots kept on disk; older ones are deleted when a new one is taken.
const SNAPSHOT_KEEP: usize = 30;

struct PendingApproval {
    task_id: Option<String>,
    answer: Sender<(bool, bool)>,
}

pub struct State {
    pub audit: Audit,
    pub tasks: TaskStore,
    pub memories: MemoryStore,
    pub snapshots: Option<Snapshotter>,
    approvals: HashMap<String, PendingApproval>,
    /// Attached UIs per task: writers that receive events.
    uis: HashMap<String, Vec<Arc<Mutex<UnixStream>>>>,
}

pub type Shared = Arc<Mutex<State>>;

/// Best-effort desktop notification through notify-send (mako etc). Silent if unavailable
/// or disabled with SLATE_NOTIFY=0.
fn notify(summary: &str, body: &str, urgency: &str) {
    if std::env::var("SLATE_NOTIFY")
        .map(|v| v == "0")
        .unwrap_or(false)
    {
        return;
    }
    let _ = std::process::Command::new("notify-send")
        .arg("-a")
        .arg("Slate")
        .arg("-u")
        .arg(urgency)
        .arg("-t")
        .arg("6000")
        .arg(summary)
        .arg(body)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

impl State {
    pub fn new(
        audit: Audit,
        tasks: TaskStore,
        memories: MemoryStore,
        snapshots: Option<Snapshotter>,
    ) -> Self {
        Self {
            audit,
            tasks,
            memories,
            snapshots,
            approvals: HashMap::new(),
            uis: HashMap::new(),
        }
    }

    fn log(&self, e: AuditEntry) {
        if let Err(err) = self.audit.append(&e) {
            slate_proto::log!("slated: audit write failed: {err:#}");
        }
    }

    fn ensure_snapshot(&mut self, task_id: &str) -> Option<String> {
        let existing = self.tasks.get(task_id).and_then(|t| t.snapshot.clone());
        if let Some(p) = existing {
            return Some(p.display().to_string());
        }
        let snap = self.snapshots.as_ref()?;
        match snap.create(task_id) {
            Ok(info) => {
                for old in snap.prune(SNAPSHOT_KEEP) {
                    slate_proto::log!("slated: pruned snapshot {}", old.display());
                }
                let p = info.path.display().to_string();
                let _ = self.tasks.update(task_id, |t| {
                    t.snapshot = Some(info.path.clone());
                });
                self.log(AuditEntry {
                    ts: now_millis(),
                    task_id: Some(task_id.into()),
                    session_id: None,
                    kind: AuditKind::Snapshot,
                    tool_name: None,
                    summary: format!("snapshot {p}"),
                    tier: None,
                    decision: None,
                    snapshot: Some(p.clone()),
                });
                Some(p)
            }
            Err(e) => {
                slate_proto::log!("slated: snapshot failed for {task_id}: {e:#}");
                self.log(AuditEntry {
                    ts: now_millis(),
                    task_id: Some(task_id.into()),
                    session_id: None,
                    kind: AuditKind::Snapshot,
                    tool_name: None,
                    summary: format!("snapshot failed: {e:#}"),
                    tier: None,
                    decision: None,
                    snapshot: None,
                });
                None
            }
        }
    }

    fn send_event(&mut self, task_id: &str, ev: &Event) {
        let Some(list) = self.uis.get_mut(task_id) else {
            return;
        };
        let line = match serde_json::to_string(ev) {
            Ok(s) => s,
            Err(_) => return,
        };
        list.retain(|w| {
            let mut g = w.lock().unwrap_or_else(|e| e.into_inner());
            g.write_all(line.as_bytes())
                .and_then(|_| g.write_all(b"\n"))
                .is_ok()
        });
    }
}

fn touched_paths(
    tool_name: &str,
    input: &serde_json::Value,
    cwd: Option<&PathBuf>,
) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(p) = input.get("file_path").and_then(|v| v.as_str()) {
        out.push(PathBuf::from(p));
    }
    if tool_name == "Bash" {
        if let Some(c) = cwd {
            out.push(c.clone());
        }
    }
    out
}

pub fn serve(path: PathBuf, state: Shared) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        // Stale socket from a previous run? Refuse if something answers.
        if UnixStream::connect(&path).is_ok() {
            anyhow::bail!("another slated is already listening on {}", path.display());
        }
        std::fs::remove_file(&path)?;
    }
    let listener =
        UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
    slate_proto::log!(
        "slated {} listening on {}",
        slate_proto::VERSION,
        path.display()
    );
    for conn in listener.incoming() {
        let Ok(stream) = conn else { continue };
        let st = Arc::clone(&state);
        std::thread::spawn(move || {
            if let Err(e) = handle_conn(stream, st) {
                slate_proto::log!("slated: connection error: {e:#}");
            }
        });
    }
    Ok(())
}

fn handle_conn(stream: UnixStream, state: Shared) -> Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let writer = Arc::new(Mutex::new(stream));
    let mut attached: Option<String> = None;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let env: Envelope = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(e) => {
                // Echo the caller's id if we can find it, so clients do not wait forever.
                let id = serde_json::from_str::<serde_json::Value>(&line)
                    .ok()
                    .and_then(|v| v.get("id").and_then(|i| i.as_str().map(str::to_string)))
                    .unwrap_or_else(|| "?".into());
                let r = ReplyEnvelope {
                    id,
                    reply: Reply::Error {
                        message: format!("bad request: {e} (is slated older than this client?)"),
                    },
                };
                write_line(&writer, &r)?;
                continue;
            }
        };
        if let Request::UiAttach { task_id } = &env.request {
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            st.uis
                .entry(task_id.clone())
                .or_default()
                .push(Arc::clone(&writer));
            attached = Some(task_id.clone());
            drop(st);
            write_line(
                &writer,
                &ReplyEnvelope {
                    id: env.id,
                    reply: Reply::Ok,
                },
            )?;
            continue;
        }
        let reply = dispatch(env.request, &state);
        write_line(&writer, &ReplyEnvelope { id: env.id, reply })?;
    }
    if let Some(task_id) = attached {
        let orphaned = {
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(list) = st.uis.get_mut(&task_id) {
                list.retain(|w| !Arc::ptr_eq(w, &writer));
            }
            let no_ui = st.uis.get(&task_id).map(|l| l.is_empty()).unwrap_or(true);
            no_ui
                && st
                    .tasks
                    .get(&task_id)
                    .map(|t| t.ended.is_none())
                    .unwrap_or(false)
        };
        // The shell that ran this task is gone mid-turn (window closed, Ctrl-C, crash):
        // end the task, or the desktop would show "working" until it aged out.
        if orphaned {
            let _ = dispatch(Request::TaskEnd { task_id, ok: false }, &state);
        }
    }
    Ok(())
}

fn write_line(w: &Arc<Mutex<UnixStream>>, r: &ReplyEnvelope) -> Result<()> {
    let mut g = w.lock().unwrap_or_else(|e| e.into_inner());
    g.write_all(serde_json::to_string(r)?.as_bytes())?;
    g.write_all(b"\n")?;
    Ok(())
}

fn dispatch(req: Request, state: &Shared) -> Reply {
    match req {
        Request::Ping => {
            let st = state.lock().unwrap_or_else(|e| e.into_inner());
            Reply::Pong {
                version: slate_proto::VERSION.into(),
                snapshots: st.snapshots.is_some(),
            }
        }
        Request::TaskStart {
            backend,
            prompt,
            cwd,
            auto_approve,
            quiet,
        } => {
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            let task_id = new_task_id();
            let task = Task {
                task_id: task_id.clone(),
                started: now_millis(),
                ended: None,
                backend,
                prompt: prompt.chars().take(500).collect(),
                cwd: cwd.clone(),
                snapshot: None,
                tool_calls: 0,
                touched: vec![cwd],
                remembered: Default::default(),
                undone: false,
                auto_approve,
                quiet,
            };
            if let Err(e) = st.tasks.insert(task.clone()) {
                return Reply::Error {
                    message: format!("{e:#}"),
                };
            }
            st.log(AuditEntry {
                ts: now_millis(),
                task_id: Some(task_id.clone()),
                session_id: None,
                kind: AuditKind::TaskStart,
                tool_name: None,
                summary: task
                    .prompt
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(120)
                    .collect(),
                tier: None,
                decision: None,
                snapshot: None,
            });
            Reply::TaskStarted { task_id }
        }
        Request::TaskEnd { task_id, ok } => {
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            let _ = st.tasks.update(&task_id, |t| t.ended = Some(now_millis()));
            // Resolve any approvals still pending for this task.
            let ids: Vec<String> = st
                .approvals
                .iter()
                .filter(|(_, p)| p.task_id.as_deref() == Some(&task_id))
                .map(|(id, _)| id.clone())
                .collect();
            for id in ids {
                if let Some(p) = st.approvals.remove(&id) {
                    let _ = p.answer.send((false, false));
                }
            }
            st.uis.remove(&task_id);
            let (calls, quiet) = st
                .tasks
                .get(&task_id)
                .map(|t| (t.tool_calls, t.quiet))
                .unwrap_or((0, false));
            if calls > 0 && !quiet {
                notify(
                    if ok {
                        "Slate finished"
                    } else {
                        "Slate stopped with an error"
                    },
                    &format!("{calls} action(s); say \"undo\" in slash to roll back file changes"),
                    "low",
                );
            }
            st.log(AuditEntry {
                ts: now_millis(),
                task_id: Some(task_id),
                session_id: None,
                kind: AuditKind::TaskEnd,
                tool_name: None,
                summary: if ok { "ok".into() } else { "error".into() },
                tier: None,
                decision: None,
                snapshot: None,
            });
            Reply::Ok
        }
        Request::ToolCheck {
            task_id,
            session_id,
            tool_name,
            tool_input,
            cwd,
        } => {
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            let verdict = policy::classify(&tool_name, &tool_input);
            let remembered = task_id
                .as_ref()
                .and_then(|id| st.tasks.get(id))
                .map(|t| t.remembered.contains(&tool_name) || t.auto_approve)
                .unwrap_or(false);
            let mut snapshot = None;
            let decision = match verdict.tier {
                Tier::Observe => Decision::Allow,
                Tier::Reversible => {
                    if let Some(id) = &task_id {
                        snapshot = st.ensure_snapshot(id);
                    }
                    Decision::Allow
                }
                Tier::Confirm => {
                    if remembered {
                        if let Some(id) = &task_id {
                            snapshot = st.ensure_snapshot(id);
                        }
                        Decision::Allow
                    } else {
                        Decision::Ask
                    }
                }
            };
            if let Some(id) = &task_id {
                let touched = touched_paths(&tool_name, &tool_input, cwd.as_ref());
                let _ = st.tasks.update(id, |t| {
                    t.tool_calls += 1;
                    for p in touched {
                        if !t.touched.contains(&p) {
                            t.touched.push(p);
                        }
                    }
                });
            }
            st.log(AuditEntry {
                ts: now_millis(),
                task_id: task_id.clone(),
                session_id,
                kind: AuditKind::ToolCheck,
                tool_name: Some(tool_name.clone()),
                summary: summarize_tool(&tool_name, &tool_input),
                tier: Some(verdict.tier),
                decision: Some(decision),
                snapshot: snapshot.clone(),
            });
            Reply::ToolChecked {
                tier: verdict.tier,
                decision,
                reason: verdict.reason,
                snapshot,
            }
        }
        Request::ToolDone {
            task_id,
            session_id,
            tool_name,
            tool_input,
            ok,
        } => {
            let st = state.lock().unwrap_or_else(|e| e.into_inner());
            st.log(AuditEntry {
                ts: now_millis(),
                task_id,
                session_id,
                kind: AuditKind::ToolDone,
                tool_name: Some(tool_name.clone()),
                summary: format!(
                    "{} {}",
                    if ok { "ok" } else { "failed" },
                    summarize_tool(&tool_name, &tool_input)
                ),
                tier: None,
                decision: None,
                snapshot: None,
            });
            Reply::Ok
        }
        Request::ApprovalRequest {
            task_id,
            session_id,
            tool_name,
            tool_input,
        } => {
            let verdict = policy::classify(&tool_name, &tool_input);
            let summary = summarize_tool(&tool_name, &tool_input);
            let auto = task_id
                .as_ref()
                .and_then(|id| {
                    state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .tasks
                        .get(id)
                        .map(|t| t.auto_approve)
                })
                .unwrap_or(false);
            // The backend consults the permission tool for some tools regardless of the
            // hook's answer (e.g. AskUserQuestion). Observe-tier calls never need a human,
            // and neither does anything in an auto-approve task.
            if verdict.tier == Tier::Observe || auto {
                let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
                st.log(AuditEntry {
                    ts: now_millis(),
                    task_id: task_id.clone(),
                    session_id,
                    kind: AuditKind::Approval,
                    tool_name: Some(tool_name),
                    summary,
                    tier: Some(verdict.tier),
                    decision: Some(Decision::Allow),
                    snapshot: None,
                });
                if auto && verdict.tier != Tier::Observe {
                    if let Some(id) = &task_id {
                        st.ensure_snapshot(id);
                    }
                }
                return Reply::Approval {
                    allow: true,
                    message: if auto {
                        "auto-approve is on for this task".into()
                    } else {
                        "observe-tier tool, allowed by policy".into()
                    },
                };
            }
            let approval_id = new_task_id();
            let (tx, rx) = channel();
            {
                let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
                let Some(tid) = task_id.clone() else {
                    return Reply::Approval {
                        allow: false,
                        message: "no Slate task attached; denied".into(),
                    };
                };
                if !st.uis.get(&tid).map(|l| !l.is_empty()).unwrap_or(false) {
                    return Reply::Approval {
                        allow: false,
                        message: "no Slate UI attached to answer; denied".into(),
                    };
                }
                st.approvals.insert(
                    approval_id.clone(),
                    PendingApproval {
                        task_id: Some(tid.clone()),
                        answer: tx,
                    },
                );
                let ev = Event::ApprovalNeeded {
                    approval_id: approval_id.clone(),
                    tool_name: tool_name.clone(),
                    summary: summary.clone(),
                    tier: verdict.tier,
                    reason: verdict.reason.clone(),
                };
                st.send_event(&tid, &ev);
                let quiet = st.tasks.get(&tid).map(|t| t.quiet).unwrap_or(false);
                if !quiet {
                    notify(
                        "Slate needs your approval",
                        &format!("{tool_name}: {summary}\nAnswer in the slash window."),
                        "critical",
                    );
                }
            }
            let (allow, remember) = rx.recv_timeout(APPROVAL_TIMEOUT).unwrap_or((false, false));
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            st.approvals.remove(&approval_id);
            if let Some(tid) = &task_id {
                st.send_event(
                    tid,
                    &Event::ApprovalResolved {
                        approval_id: approval_id.clone(),
                    },
                );
                if allow {
                    if remember {
                        let _ = st.tasks.update(tid, |t| {
                            t.remembered.insert(tool_name.clone());
                        });
                    }
                    st.ensure_snapshot(tid);
                }
            }
            st.log(AuditEntry {
                ts: now_millis(),
                task_id: task_id.clone(),
                session_id,
                kind: AuditKind::Approval,
                tool_name: Some(tool_name),
                summary,
                tier: Some(verdict.tier),
                decision: Some(if allow {
                    Decision::Allow
                } else {
                    Decision::Deny
                }),
                snapshot: None,
            });
            Reply::Approval {
                allow,
                message: if allow {
                    "approved by user".into()
                } else {
                    "denied by user".into()
                },
            }
        }
        Request::ApprovalAnswer {
            approval_id,
            allow,
            remember,
        } => {
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            match st.approvals.remove(&approval_id) {
                Some(p) => {
                    let _ = p.answer.send((allow, remember));
                    Reply::Ok
                }
                None => Reply::Error {
                    message: "no such pending approval".into(),
                },
            }
        }
        Request::UiAttach { .. } => Reply::Error {
            message: "attach handled at connection level".into(),
        },
        Request::Undo { task_id } => run_undo(state, task_id, false),
        Request::UndoPreview { task_id } => run_undo(state, task_id, true),
        Request::AuditTail { n } => {
            let st = state.lock().unwrap_or_else(|e| e.into_inner());
            match st.audit.tail(n) {
                Ok(entries) => Reply::Audit { entries },
                Err(e) => Reply::Error {
                    message: format!("{e:#}"),
                },
            }
        }
        Request::Tasks { n } => {
            let st = state.lock().unwrap_or_else(|e| e.into_inner());
            Reply::Tasks {
                tasks: st.tasks.recent(n),
            }
        }
        Request::MemoryAdd { text, task_id } => {
            let st = state.lock().unwrap_or_else(|e| e.into_inner());
            if text.trim().is_empty() {
                return Reply::Error {
                    message: "nothing to remember".into(),
                };
            }
            match st.memories.add(text, task_id) {
                Ok(m) => Reply::MemoryAdded { memory_id: m.id },
                Err(e) => Reply::Error {
                    message: format!("{e:#}"),
                },
            }
        }
        Request::MemoryList { n, query } => {
            let st = state.lock().unwrap_or_else(|e| e.into_inner());
            match st.memories.list(n, query.as_deref()) {
                Ok(memories) => Reply::Memories { memories },
                Err(e) => Reply::Error {
                    message: format!("{e:#}"),
                },
            }
        }
        Request::MemoryForget { memory_id } => {
            let st = state.lock().unwrap_or_else(|e| e.into_inner());
            match st.memories.forget(&memory_id) {
                Ok(true) => Reply::Ok,
                Ok(false) => Reply::Error {
                    message: format!("no memory {memory_id}"),
                },
                Err(e) => Reply::Error {
                    message: format!("{e:#}"),
                },
            }
        }
    }
}

fn run_undo(state: &Shared, task_id: Option<String>, preview: bool) -> Reply {
    let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
    let Some(snapper) = st.snapshots.clone() else {
        return Reply::Error {
            message: "snapshots are not available on this system (home is not a btrfs subvolume, or btrfs is not permitted)".into(),
        };
    };
    let task = match &task_id {
        Some(id) => st.tasks.get(id).cloned(),
        None => st.tasks.last_undoable().cloned(),
    };
    let Some(task) = task else {
        return Reply::Error {
            message: "nothing to undo".into(),
        };
    };
    let Some(path) = task.snapshot.clone() else {
        return Reply::Error {
            message: format!("task {} has no snapshot", task.task_id),
        };
    };
    if task.undone {
        return Reply::Error {
            message: format!("task {} was already undone", task.task_id),
        };
    }
    let snap = SnapshotInfo { path };
    let scope: Vec<PathBuf> = task
        .touched
        .iter()
        .map(|p| {
            if p.is_dir() {
                p.clone()
            } else {
                p.parent().map(|x| x.to_path_buf()).unwrap_or(p.clone())
            }
        })
        .collect();
    let plan = match snapper.plan(&snap, &scope) {
        Ok(p) => p,
        Err(e) => {
            return Reply::Error {
                message: format!("{e:#}"),
            }
        }
    };
    let notes = if plan.notes.is_empty() {
        String::new()
    } else {
        format!("; {}", plan.notes.join("; "))
    };
    if preview {
        return Reply::UndoResult {
            task_id: task.task_id,
            restored: plan
                .restore
                .iter()
                .chain(plan.recreate.iter())
                .cloned()
                .collect(),
            deleted: plan.delete.clone(),
            note: format!("preview{notes}"),
        };
    }
    let (restored, deleted) = match snapper.apply(&snap, &plan) {
        Ok(r) => r,
        Err(e) => {
            return Reply::Error {
                message: format!("{e:#}"),
            }
        }
    };
    let _ = st.tasks.update(&task.task_id, |t| t.undone = true);
    st.log(AuditEntry {
        ts: now_millis(),
        task_id: Some(task.task_id.clone()),
        session_id: None,
        kind: AuditKind::Undo,
        tool_name: None,
        summary: format!("restored {} deleted {}", restored.len(), deleted.len()),
        tier: None,
        decision: None,
        snapshot: Some(snap.path.display().to_string()),
    });
    Reply::UndoResult {
        task_id: task.task_id,
        restored,
        deleted,
        note: format!("applied{notes}"),
    }
}
