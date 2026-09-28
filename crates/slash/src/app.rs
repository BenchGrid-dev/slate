//! The interactive loop.

use crate::backend::{self, Backend, Event, TurnRequest};
use crate::config::Config;
use crate::daemon::{self, Attachment, Daemon};
use crate::render::{self, bold, cyan, dim, green, red, yellow};
use crate::router::{route, Input};
use crate::session::Session;
use crate::shell::ShellRunner;
use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::{Config as RlConfig, DefaultEditor};
use std::path::PathBuf;

pub struct App {
    cfg: Config,
    backend: Box<dyn Backend>,
    shell: ShellRunner,
    session: Session,
    verbose: bool,
    daemon: Option<Daemon>,
    /// `/auto on`: Confirm-tier actions run without asking for the rest of the session.
    auto_approve: bool,
    slate_bin: PathBuf,
    /// `slate-desktop`, when we are on a Wayland desktop and the binary exists.
    desktop_bin: Option<PathBuf>,
}

impl App {
    pub fn new(cfg: Config) -> Result<Self> {
        let backend = backend::by_name(&cfg.backend, &cfg)
            .ok_or_else(|| anyhow::anyhow!("unknown backend {:?} in config", cfg.backend))?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let shell = ShellRunner::new(cfg.shell(), cfg.shell_interactive, cwd);
        let session = Session::new(cfg.context_commands);
        let daemon = if cfg.slated.enable {
            match Daemon::connect(cfg.slated.auto_start) {
                Ok(d) => Some(d),
                Err(e) => {
                    eprintln!(
                        "{} slated unavailable ({e:#}); running without approvals/undo",
                        yellow("warning:")
                    );
                    None
                }
            }
        } else {
            None
        };
        let auto_approve = cfg.auto_approve;
        Ok(Self {
            auto_approve,
            cfg,
            backend,
            shell,
            session,
            verbose: false,
            daemon,
            slate_bin: daemon::sibling_bin("slate"),
            desktop_bin: std::env::var_os("WAYLAND_DISPLAY")
                .map(|_| daemon::sibling_bin("slate-desktop"))
                .filter(|p| p.is_absolute() && p.exists()),
        })
    }

    fn daemon(&mut self) -> Option<&mut Daemon> {
        // Reconnect lazily if the daemon went away.
        if self.daemon.is_none() && self.cfg.slated.enable {
            self.daemon = Daemon::connect(self.cfg.slated.auto_start).ok();
        }
        self.daemon.as_mut()
    }

    fn prompt(&self) -> String {
        let cwd = self.shell.cwd();
        let shown = match dirs::home_dir() {
            Some(h) if cwd == h => "~".to_string(),
            Some(h) if cwd.starts_with(&h) => format!(
                "~/{}",
                cwd.strip_prefix(&h)
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ),
            _ => cwd.display().to_string(),
        };
        let mark = if self.auto_approve {
            yellow("⚡")
        } else {
            cyan("❯")
        };
        format!("{} {} ", dim(&shown), mark)
    }

    fn set_title(&self, state: &str) {
        // OSC 0: terminal title. mako's click action focuses the window titled "slash…".
        print!("\x1b]0;slash {state}\x07");
        render::flush();
    }

    pub fn run(&mut self) -> Result<i32> {
        self.set_title("· ready");
        let slated_note = match self.daemon.as_mut() {
            Some(d) => {
                if d.snapshots_enabled() {
                    "slated: approvals, audit, undo"
                } else {
                    "slated: approvals, audit (no snapshots)"
                }
            }
            None => "slated: off",
        };
        let desktop_note = if self.desktop_bin.is_some() {
            "  ·  desktop: on"
        } else {
            ""
        };
        self.warn_if_updated();
        println!(
            "{} {}  {}",
            bold("slash"),
            dim(slate_proto::VERSION),
            dim(&format!(
                "backend: {}{}  ·  {slated_note}{desktop_note}  ·  /help",
                self.backend.name(),
                self.backend
                    .model()
                    .map(|m| format!(" ({m})"))
                    .unwrap_or_default()
            ))
        );
        let rl_cfg = RlConfig::builder().auto_add_history(true).build();
        let mut rl = DefaultEditor::with_config(rl_cfg)?;
        let hist = history_path();
        if let Some(h) = &hist {
            let _ = rl.load_history(h);
        }

        loop {
            match rl.readline(&self.prompt()) {
                Ok(line) => {
                    if let Some(code) = self.handle(&line) {
                        if let Some(h) = &hist {
                            let _ = rl.save_history(h);
                        }
                        return Ok(code);
                    }
                    if let Some(h) = &hist {
                        let _ = rl.save_history(h);
                    }
                }
                Err(ReadlineError::Interrupted) => continue,
                Err(ReadlineError::Eof) => return Ok(0),
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Returns Some(exit_code) when the shell should exit.
    fn handle(&mut self, line: &str) -> Option<i32> {
        match route(line) {
            Input::Empty => None,
            Input::Shell(cmd) => {
                match self.shell.run(&cmd) {
                    Ok(rec) => {
                        if rec.output_ended_without_newline {
                            println!("{}", dim("⏎"));
                        }
                        if let Some(c) = rec.exit_code {
                            if c != 0 {
                                println!("{}", dim(&format!("exit {c}")));
                            }
                        }
                        self.session.push(rec);
                        let _ = std::env::set_current_dir(self.shell.cwd());
                    }
                    Err(e) => println!("{} {e:#}", red("error:")),
                }
                None
            }
            Input::Control { name, args } => self.control(&name, &args),
            Input::Agent(text) => {
                self.agent_turn(&text);
                None
            }
        }
    }

    fn control(&mut self, name: &str, args: &str) -> Option<i32> {
        match name {
            "help" | "?" => {
                println!("{}", bold("slash"));
                println!("  {}   talk to the agent", dim("<text>"));
                println!("  {}   run in {}", dim("!<cmd>"), self.cfg.shell());
                println!(
                    "  {}   escape a leading slash for the agent",
                    dim("//<text>")
                );
                println!();
                println!("  /agent [claude|codex]   show or switch backend (starts a new session)");
                println!(
                    "  /model [name|default]   show or switch model (sonnet, opus, or a full id)"
                );
                println!("  /new                    start a new agent session");
                println!("  /session                show the backend session id");
                println!(
                    "  /context                show what the agent is told about this session"
                );
                println!("  /history                manual commands run in this session");
                println!("  /cd <dir>               change directory");
                println!("  /undo [--preview]       roll back the last task's file changes (needs slated + btrfs)");
                println!("  /audit [N]              recent audit entries");
                println!("  /tasks                  recent tasks");
                println!("  /remember <text>        store a memory; /memories [query] lists them");
                println!("  /auto [on|off]          bypass approvals: Confirm-tier actions run without asking (still audited, still snapshotted)");
                println!("  /verbose                toggle raw event output");
                println!("  /quit, /exit            leave slash");
                println!();
                println!(
                    "  {}",
                    dim("anything else starting with / is passed to the agent backend")
                );
                None
            }
            "agent" => {
                if args.is_empty() {
                    println!("backend: {}", self.backend.name());
                } else {
                    match backend::by_name(args, &self.cfg) {
                        Some(b) => {
                            self.backend = b;
                            self.session.reset_sent();
                            println!("backend: {} (new session)", self.backend.name());
                        }
                        None => println!(
                            "{} unknown backend {args:?}; try claude or codex",
                            red("error:")
                        ),
                    }
                }
                None
            }
            "model" => {
                if args.is_empty() {
                    match self.backend.model() {
                        Some(m) => println!("model: {m}"),
                        None => println!("model: {}", dim("backend default")),
                    }
                } else if args == "default" {
                    self.backend.set_model(None);
                    println!("model: {}", dim("backend default"));
                } else {
                    self.backend.set_model(Some(args.to_string()));
                    println!("model: {args}");
                }
                None
            }
            "new" => {
                self.backend.reset();
                self.session.reset_sent();
                println!("{}", dim("new session"));
                None
            }
            "session" => {
                match self.backend.session_id() {
                    Some(id) => println!("{} session {id}", self.backend.name()),
                    None => println!("{}", dim("no session yet")),
                }
                None
            }
            "context" => {
                println!("{}", dim(&self.session.context_for_agent(self.shell.cwd())));
                None
            }
            "history" => {
                for r in self.session.records() {
                    let code = r
                        .exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "sig".into());
                    println!(
                        "{} {}  {}",
                        dim(&format!("[{code}]")),
                        r.command,
                        dim(&r.cwd.display().to_string())
                    );
                }
                None
            }
            "cd" => {
                let target = if args.is_empty() {
                    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
                } else {
                    let p = PathBuf::from(shellexpand_home(args));
                    if p.is_absolute() {
                        p
                    } else {
                        self.shell.cwd().join(p)
                    }
                };
                match target.canonicalize() {
                    Ok(p) if p.is_dir() => {
                        self.shell.set_cwd(p.clone());
                        let _ = std::env::set_current_dir(&p);
                    }
                    _ => println!("{} no such directory: {}", red("error:"), target.display()),
                }
                None
            }
            "undo" => {
                let preview = args.contains("--preview") || args.contains("-n");
                let id = args
                    .split_whitespace()
                    .find(|a| !a.starts_with('-'))
                    .map(str::to_string);
                let Some(d) = self.daemon() else {
                    println!("{} slated is not available", red("error:"));
                    return None;
                };
                let req = if preview {
                    slate_proto::Request::UndoPreview { task_id: id }
                } else {
                    slate_proto::Request::Undo { task_id: id }
                };
                match d.call(req) {
                    Ok(slate_proto::Reply::UndoResult {
                        task_id,
                        restored,
                        deleted,
                        note,
                    }) => {
                        println!("{} task {} ({note})", green("↶"), dim(&task_id));
                        for p in &restored {
                            println!("  {} {}", dim("restore"), p.display());
                        }
                        for p in &deleted {
                            println!("  {} {}", dim("delete "), p.display());
                        }
                        if restored.is_empty() && deleted.is_empty() {
                            println!("  {}", dim("nothing changed since the snapshot"));
                        }
                    }
                    Ok(slate_proto::Reply::Error { message }) => {
                        println!("{} {message}", red("error:"))
                    }
                    Ok(other) => println!("{} unexpected reply {other:?}", red("error:")),
                    Err(e) => println!("{} {e:#}", red("error:")),
                }
                None
            }
            "audit" => {
                let n = args.trim().parse().unwrap_or(20);
                let Some(d) = self.daemon() else {
                    println!("{} slated is not available", red("error:"));
                    return None;
                };
                match d.call(slate_proto::Request::AuditTail { n }) {
                    Ok(slate_proto::Reply::Audit { entries }) => {
                        for e in entries {
                            let tier = e.tier.map(|t| t.as_str()).unwrap_or("");
                            let dec = e
                                .decision
                                .map(|d| format!("{d:?}").to_lowercase())
                                .unwrap_or_default();
                            println!(
                                "{} {:<10} {:<10} {:<5} {} {}",
                                dim(&format!("{:?}", e.kind).to_lowercase()),
                                e.tool_name.unwrap_or_default(),
                                dim(tier),
                                dim(&dec),
                                e.summary,
                                dim(e.snapshot.map(|_| "📸").unwrap_or_default())
                            );
                        }
                    }
                    Ok(other) => println!("{} unexpected reply {other:?}", red("error:")),
                    Err(e) => println!("{} {e:#}", red("error:")),
                }
                None
            }
            "tasks" => {
                let Some(d) = self.daemon() else {
                    println!("{} slated is not available", red("error:"));
                    return None;
                };
                match d.call(slate_proto::Request::Tasks { n: 10 }) {
                    Ok(slate_proto::Reply::Tasks { tasks }) => {
                        for t in tasks {
                            println!(
                                "{} {:?} calls={} snapshot={} {}",
                                dim(&t.task_id),
                                t.backend,
                                t.tool_calls,
                                if t.snapshot.is_some() { "yes" } else { "no" },
                                t.prompt.lines().next().unwrap_or("")
                            );
                        }
                    }
                    Ok(other) => println!("{} unexpected reply {other:?}", red("error:")),
                    Err(e) => println!("{} {e:#}", red("error:")),
                }
                None
            }
            "remember" => {
                let Some(d) = self.daemon() else {
                    println!("{} slated is not available", red("error:"));
                    return None;
                };
                match d.call(slate_proto::Request::MemoryAdd {
                    text: args.to_string(),
                    task_id: None,
                }) {
                    Ok(slate_proto::Reply::MemoryAdded { .. }) => println!("{}", dim("remembered")),
                    Ok(slate_proto::Reply::Error { message }) => {
                        println!("{} {message}", red("error:"))
                    }
                    Ok(other) => println!("{} unexpected reply {other:?}", red("error:")),
                    Err(e) => println!("{} {e:#}", red("error:")),
                }
                None
            }
            "memories" => {
                let Some(d) = self.daemon() else {
                    println!("{} slated is not available", red("error:"));
                    return None;
                };
                let query = if args.trim().is_empty() {
                    None
                } else {
                    Some(args.trim().to_string())
                };
                match d.call(slate_proto::Request::MemoryList { n: 50, query }) {
                    Ok(slate_proto::Reply::Memories { memories }) => {
                        if memories.is_empty() {
                            println!("{}", dim("no memories"));
                        }
                        for m in memories {
                            println!("{} {}", dim(&m.id), m.text);
                        }
                    }
                    Ok(other) => println!("{} unexpected reply {other:?}", red("error:")),
                    Err(e) => println!("{} {e:#}", red("error:")),
                }
                None
            }
            "auto" => {
                match args.trim() {
                    "on" | "1" | "true" => self.auto_approve = true,
                    "off" | "0" | "false" => self.auto_approve = false,
                    "" => {}
                    other => {
                        println!("{} /auto on|off (got {other:?})", red("error:"));
                        return None;
                    }
                }
                println!(
                    "{}",
                    if self.auto_approve {
                        yellow("⚡ auto-approve on: the agent will not ask before Confirm-tier actions (every action is still audited and file changes snapshotted)")
                    } else {
                        dim("auto-approve off: Confirm-tier actions ask first")
                    }
                );
                None
            }
            "verbose" => {
                self.verbose = !self.verbose;
                println!(
                    "{}",
                    dim(&format!(
                        "verbose {}",
                        if self.verbose { "on" } else { "off" }
                    ))
                );
                None
            }
            "quit" | "exit" | "q" => Some(0),
            _ => {
                // Unknown: pass through to the backend as-is (e.g. /compact for Claude Code).
                let full = if args.is_empty() {
                    format!("/{name}")
                } else {
                    format!("/{name} {args}")
                };
                self.agent_turn(&full);
                None
            }
        }
    }

    /// Installed Slate newer than this process? Say so: a running slash keeps its own
    /// version while updated tools are already on PATH.
    fn warn_if_updated(&self) {
        if let Some(v) = daemon::binary_version(&self.slate_bin) {
            if v != slate_proto::VERSION {
                println!(
                    "{} Slate {v} is installed but this slash is {}; type exit and start slash again to use it.",
                    yellow("note:"),
                    slate_proto::VERSION
                );
            }
        }
    }

    fn agent_turn(&mut self, prompt: &str) {
        self.warn_if_updated();
        let context = Session::instructions().to_string();
        let first_turn = self.backend.session_id().is_none();
        if first_turn {
            self.session.reset_sent();
        }
        let memories = match self.daemon() {
            Some(d) => d.memories(30),
            None => vec![],
        };
        let cwd_now = self.shell.cwd().to_path_buf();
        let delta = self
            .session
            .delta_for_agent(&cwd_now, &memories, first_turn);
        let full_prompt = if delta.is_empty() {
            prompt.to_string()
        } else {
            format!("<slash-context>\n{delta}</slash-context>\n\n{prompt}")
        };
        let prompt = full_prompt.as_str();
        let cwd = self.shell.cwd().to_path_buf();
        let verbose = self.verbose;
        let backend_name = self.backend.name();
        let auto_approve = self.auto_approve;
        let task_id = match self.daemon() {
            Some(d) => match d.task_start(backend_name, prompt, &cwd, auto_approve) {
                Ok(id) => Some(id),
                Err(e) => {
                    println!("{} slated: {e:#}", yellow("warning:"));
                    None
                }
            },
            None => None,
        };
        let attachment = task_id.as_deref().and_then(|id| Attachment::start(id).ok());
        let slate_bin = self.slate_bin.clone();
        let desktop_bin = self.desktop_bin.clone();
        let mut last_text: Option<String> = None;
        let mut streamed = 0usize;
        let mut on_event = |ev: Event| {
            match ev {
                Event::TextDelta(t) => {
                    print!("{t}");
                    streamed += t.len();
                }
                Event::SessionStarted(id) => {
                    if verbose {
                        println!("{}", dim(&format!("session {id}")));
                    }
                }
                Event::Text(t) => {
                    if streamed > 0 {
                        // Already printed incrementally; just end the line.
                        println!();
                        streamed = 0;
                    } else {
                        println!("{t}");
                    }
                    last_text = Some(t);
                }
                Event::ToolStart { name, detail } => {
                    println!("{}", render::tool_line(&name, &detail));
                }
                Event::ToolEnd { name, ok, detail } => {
                    if !ok {
                        println!(
                            "  {} {} {}",
                            red("✗"),
                            cyan(&name),
                            dim(&render::truncate(detail.lines().next().unwrap_or(""), 100))
                        );
                    } else if verbose && !detail.trim().is_empty() {
                        for l in detail.lines().take(20) {
                            println!("    {}", dim(l));
                        }
                    }
                }
                Event::Done { ok, summary, stats } => {
                    if let Some(s) = summary {
                        let already_shown = ok && last_text.as_deref() == Some(s.as_str());
                        if !already_shown && !s.trim().is_empty() {
                            println!("{s}");
                        }
                    }
                    let mark = if ok { green("✓") } else { red("✗") };
                    match stats {
                        Some(st) => println!("{} {}", mark, dim(&st)),
                        None => println!("{mark}"),
                    }
                }
                Event::Other(s) => {
                    if verbose {
                        println!("{}", dim(&s));
                    }
                }
            }
            render::flush();
        };
        self.set_title("▸ working");
        let req = TurnRequest {
            prompt,
            context: &context,
            cwd: &cwd,
            task_id: task_id.as_deref(),
            slate_bin: task_id.as_ref().map(|_| slate_bin.as_path()),
            desktop_bin: desktop_bin.as_deref(),
        };
        let result = self.backend.run_turn(req, &mut on_event);
        let ok = result.is_ok();
        if let Err(e) = result {
            println!("{} {e:#}", red("error:"));
            println!(
                "{}",
                yellow("hint: check the backend is installed and logged in; /agent to switch")
            );
        }
        if let Some(a) = attachment {
            a.stop();
        }
        self.set_title("· ready");
        if let Some(id) = task_id {
            if let Some(d) = self.daemon() {
                d.task_end(&id, ok);
            }
        }
    }
}

fn shellexpand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~") {
        if let Some(h) = dirs::home_dir() {
            return format!("{}{}", h.display(), rest);
        }
    }
    p.to_string()
}

fn history_path() -> Option<PathBuf> {
    let dir = dirs::state_dir()
        .or_else(dirs::data_local_dir)?
        .join("slate");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("slash_history"))
}
