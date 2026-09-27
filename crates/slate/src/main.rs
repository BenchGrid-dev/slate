//! slate: CLI for slated, plus the hook and MCP entry points backends call.

mod client;
mod hook;
mod mcp;

use anyhow::{bail, Result};
use client::Client;
use slate_proto::{AuditKind, Reply, Request};

fn usage() -> ! {
    eprintln!(
        "slate {}

usage:
  slate status                 daemon version and whether snapshots work
  slate audit [N]              last N audit entries (default 30)
  slate tasks [N]              recent tasks
  slate undo [--preview] [ID]  roll back the last (or given) task's file changes
  slate remember TEXT          store a memory; slate memories [QUERY] lists them; slate forget ID
  slate skills install [DIR]   link OS Skills (default: the bundled base set) into ~/.claude/skills
  slate skills list
  slate hook pre-tool-use      Claude Code PreToolUse hook (reads stdin)
  slate hook post-tool-use     Claude Code PostToolUse hook (reads stdin)
  slate mcp                    MCP server exposing the approval tool",
        slate_proto::VERSION
    );
    std::process::exit(2)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    match cmd {
        "status" => {
            let mut c = Client::connect()?;
            match c.call(Request::Ping)? {
                Reply::Pong { version, snapshots } => {
                    println!(
                        "slated {version} at {}",
                        slate_proto::socket_path().display()
                    );
                    println!(
                        "snapshots: {}",
                        if snapshots { "enabled" } else { "unavailable" }
                    );
                }
                other => bail!("unexpected reply: {other:?}"),
            }
        }
        "audit" => {
            let n = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
            let mut c = Client::connect()?;
            match c.call(Request::AuditTail { n })? {
                Reply::Audit { entries } => {
                    for e in entries {
                        let kind = match e.kind {
                            AuditKind::TaskStart => "task ▶",
                            AuditKind::TaskEnd => "task ■",
                            AuditKind::ToolCheck => "check",
                            AuditKind::ToolDone => "done ",
                            AuditKind::Approval => "ask  ",
                            AuditKind::Snapshot => "snap ",
                            AuditKind::Undo => "undo ",
                        };
                        let tier = e.tier.map(|t| t.as_str()).unwrap_or("-");
                        let dec = e
                            .decision
                            .map(|d| format!("{d:?}").to_lowercase())
                            .unwrap_or_default();
                        println!(
                            "{} {kind} {:<10} {:<6} {:<10} {}",
                            fmt_ts(e.ts),
                            tier,
                            dec,
                            e.tool_name.unwrap_or_default(),
                            e.summary
                        );
                    }
                }
                other => bail!("unexpected reply: {other:?}"),
            }
        }
        "tasks" => {
            let n = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
            let mut c = Client::connect()?;
            match c.call(Request::Tasks { n })? {
                Reply::Tasks { tasks } => {
                    for t in tasks {
                        println!(
                            "{} {} {:?} calls={} snap={} {}",
                            fmt_ts(t.started),
                            t.task_id,
                            t.backend,
                            t.tool_calls,
                            t.snapshot.map(|_| "yes").unwrap_or("no"),
                            t.prompt.lines().next().unwrap_or("")
                        );
                    }
                }
                other => bail!("unexpected reply: {other:?}"),
            }
        }
        "undo" => {
            let preview = args.iter().any(|a| a == "--preview" || a == "-n");
            let id = args.iter().skip(1).find(|a| !a.starts_with('-')).cloned();
            let mut c = Client::connect()?;
            let req = if preview {
                Request::UndoPreview { task_id: id }
            } else {
                Request::Undo { task_id: id }
            };
            match c.call(req)? {
                Reply::UndoResult {
                    task_id,
                    restored,
                    deleted,
                    note,
                } => {
                    println!("task {task_id} ({note})");
                    for p in &restored {
                        println!("  restore {}", p.display());
                    }
                    for p in &deleted {
                        println!("  delete  {}", p.display());
                    }
                    if restored.is_empty() && deleted.is_empty() {
                        println!("  nothing changed since the snapshot");
                    }
                }
                Reply::Error { message } => bail!("{message}"),
                other => bail!("unexpected reply: {other:?}"),
            }
        }
        "remember" => {
            let text = args[1..].join(" ");
            let mut c = Client::connect()?;
            match c.call(Request::MemoryAdd {
                text,
                task_id: None,
            })? {
                Reply::MemoryAdded { memory_id } => println!("remembered ({memory_id})"),
                Reply::Error { message } => bail!("{message}"),
                other => bail!("unexpected reply: {other:?}"),
            }
        }
        "memories" => {
            let query = args.get(1).cloned();
            let mut c = Client::connect()?;
            match c.call(Request::MemoryList { n: 50, query })? {
                Reply::Memories { memories } => {
                    for m in memories {
                        println!("{} {}  {}", fmt_ts(m.ts), m.id, m.text);
                    }
                }
                other => bail!("unexpected reply: {other:?}"),
            }
        }
        "forget" => {
            let id = args.get(1).cloned().unwrap_or_default();
            let mut c = Client::connect()?;
            match c.call(Request::MemoryForget { id })? {
                Reply::Ok => println!("forgotten"),
                Reply::Error { message } => bail!("{message}"),
                other => bail!("unexpected reply: {other:?}"),
            }
        }
        "skills" => match args.get(1).map(String::as_str) {
            Some("install") => skills_install(args.get(2).map(std::path::PathBuf::from))?,
            Some("list") => skills_list()?,
            _ => usage(),
        },
        "hook" => match args.get(1).map(String::as_str) {
            Some("pre-tool-use") => hook::pre_tool_use()?,
            Some("post-tool-use") => hook::post_tool_use()?,
            _ => usage(),
        },
        "mcp" => mcp::serve()?,
        "--version" | "-V" => println!("slate {}", slate_proto::VERSION),
        _ => usage(),
    }
    Ok(())
}

fn fmt_ts(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}")
}

/// Where the bundled skills live: next to the binary (installed) or in the repo (dev).
fn bundled_skills_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut candidates = vec![];
    if let Some(dir) = exe.parent() {
        candidates.push(dir.join("../share/slate/skills"));
        candidates.push(dir.join("../../skills/base")); // target/debug/slate -> repo
    }
    if let Some(p) = std::env::var_os("SLATE_SKILLS_DIR") {
        candidates.insert(0, std::path::PathBuf::from(p));
    }
    candidates
        .into_iter()
        .find(|p| p.is_dir())
        .and_then(|p| p.canonicalize().ok())
}

fn claude_skills_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join(".claude")
            .join("skills"),
    )
}

fn skills_install(dir: Option<std::path::PathBuf>) -> Result<()> {
    let src = match dir {
        Some(d) => d.canonicalize()?,
        None => bundled_skills_dir().ok_or_else(|| {
            anyhow::anyhow!("no bundled skills found; pass a directory or set SLATE_SKILLS_DIR")
        })?,
    };
    let dst_root = claude_skills_dir().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    std::fs::create_dir_all(&dst_root)?;
    let mut n = 0;
    for entry in std::fs::read_dir(&src)? {
        let entry = entry?;
        let p = entry.path();
        if !p.join("SKILL.md").is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let dst = dst_root.join(format!("slate-{name}"));
        if dst.is_symlink() || dst.exists() {
            if dst.is_symlink() {
                std::fs::remove_file(&dst)?;
            } else {
                println!("skip {} (exists and is not a symlink)", dst.display());
                continue;
            }
        }
        std::os::unix::fs::symlink(&p, &dst)?;
        println!("linked {} -> {}", dst.display(), p.display());
        n += 1;
    }
    println!("{n} skills installed from {}", src.display());
    Ok(())
}

fn skills_list() -> Result<()> {
    let Some(dst_root) = claude_skills_dir() else {
        bail!("HOME is not set");
    };
    let Ok(rd) = std::fs::read_dir(&dst_root) else {
        println!(
            "no skills installed ({} does not exist)",
            dst_root.display()
        );
        return Ok(());
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(rest) = name.strip_prefix("slate-") {
            let target = std::fs::read_link(e.path())
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            println!("{rest:<20} {target}");
        }
    }
    Ok(())
}
