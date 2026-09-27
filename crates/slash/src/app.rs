//! The interactive loop.

use crate::backend::{self, Backend, Event, TurnRequest};
use crate::config::Config;
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
}

impl App {
    pub fn new(cfg: Config) -> Result<Self> {
        let backend = backend::by_name(&cfg.backend, &cfg)
            .ok_or_else(|| anyhow::anyhow!("unknown backend {:?} in config", cfg.backend))?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let shell = ShellRunner::new(cfg.shell(), cfg.shell_interactive, cwd);
        let session = Session::new(cfg.context_commands);
        Ok(Self {
            cfg,
            backend,
            shell,
            session,
            verbose: false,
        })
    }

    fn prompt(&self) -> String {
        let cwd = self.shell.cwd();
        let home = dirs::home_dir();
        let shown = match &home {
            Some(h) if cwd.starts_with(h) => {
                format!(
                    "~{}",
                    cwd.strip_prefix(h)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                )
            }
            _ => cwd.display().to_string(),
        };
        let shown = if shown == "~" || shown.is_empty() {
            "~".to_string()
        } else {
            shown.trim_end_matches('/').to_string()
        };
        format!("{} {} ", dim(&shown), cyan("❯"))
    }

    pub fn run(&mut self) -> Result<i32> {
        println!(
            "{} {}  {}",
            bold("slash"),
            dim(slate_proto::VERSION),
            dim(&format!(
                "backend: {}  ·  /help for commands",
                self.backend.name()
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

    fn agent_turn(&mut self, prompt: &str) {
        let context = self.session.context_for_agent(self.shell.cwd());
        let cwd = self.shell.cwd().to_path_buf();
        let verbose = self.verbose;
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
        let req = TurnRequest {
            prompt,
            context: &context,
            cwd: &cwd,
        };
        if let Err(e) = self.backend.run_turn(req, &mut on_event) {
            println!("{} {e:#}", red("error:"));
            println!(
                "{}",
                yellow("hint: check the backend is installed and logged in; /agent to switch")
            );
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
