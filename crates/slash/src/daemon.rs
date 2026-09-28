//! slash's connection to slated: task lifecycle and the approval UI.

use crate::render::{bold, cyan, dim, red, yellow};
use anyhow::{bail, Context, Result};
use slate_proto::{Backend, Envelope, Event, Reply, ReplyEnvelope, Request};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct Daemon {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl Daemon {
    fn open() -> Result<Self> {
        let stream = UnixStream::connect(slate_proto::socket_path())?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            next_id: 1,
        })
    }

    /// Connect, starting slated if needed and allowed.
    pub fn connect(auto_start: bool) -> Result<Self> {
        if let Ok(d) = Self::open() {
            return Ok(d);
        }
        if !auto_start {
            bail!("slated is not running");
        }
        let bin = sibling_bin("slated");
        let log = slate_proto::state_dir().join("slated.log");
        let _ = std::fs::create_dir_all(slate_proto::state_dir());
        let logf = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log);
        let mut cmd = std::process::Command::new(&bin);
        cmd.stdin(std::process::Stdio::null());
        match logf {
            Ok(f) => {
                cmd.stdout(f.try_clone()?).stderr(f);
            }
            Err(_) => {
                cmd.stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
            }
        }
        cmd.spawn()
            .with_context(|| format!("starting {}", bin.display()))?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(d) = Self::open() {
                return Ok(d);
            }
        }
        bail!(
            "started {} but it did not come up (see {})",
            bin.display(),
            log.display()
        )
    }

    pub fn call(&mut self, request: Request) -> Result<Reply> {
        let id = self.next_id.to_string();
        self.next_id += 1;
        let env = Envelope {
            id: id.clone(),
            request,
        };
        self.stream
            .write_all(serde_json::to_string(&env)?.as_bytes())?;
        self.stream.write_all(b"\n")?;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                bail!("slated closed the connection");
            }
            if let Ok(r) = serde_json::from_str::<ReplyEnvelope>(&line) {
                if r.id == id || r.id == "?" {
                    return Ok(r.reply);
                }
            }
        }
    }

    pub fn task_start(&mut self, backend: &str, prompt: &str, cwd: &Path) -> Result<String> {
        let backend = match backend {
            "claude" => Backend::Claude,
            "codex" => Backend::Codex,
            other => Backend::Other(other.into()),
        };
        match self.call(Request::TaskStart {
            backend,
            prompt: prompt.into(),
            cwd: cwd.to_path_buf(),
        })? {
            Reply::TaskStarted { task_id } => Ok(task_id),
            Reply::Error { message } => bail!("{message}"),
            other => bail!("unexpected reply {other:?}"),
        }
    }

    pub fn task_end(&mut self, task_id: &str, ok: bool) {
        let _ = self.call(Request::TaskEnd {
            task_id: task_id.into(),
            ok,
        });
    }

    /// Recent memories, oldest first.
    pub fn memories(&mut self, n: usize) -> Vec<String> {
        match self.call(Request::MemoryList { n, query: None }) {
            Ok(Reply::Memories { memories }) => memories.into_iter().map(|m| m.text).collect(),
            _ => vec![],
        }
    }

    pub fn snapshots_enabled(&mut self) -> bool {
        matches!(
            self.call(Request::Ping),
            Ok(Reply::Pong {
                snapshots: true,
                ..
            })
        )
    }
}

/// A second connection that receives approval events for one task and asks the
/// human at the terminal. Runs on its own thread until `Attachment::stop`.
pub struct Attachment {
    stream: UnixStream,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Attachment {
    pub fn start(task_id: &str) -> Result<Self> {
        let mut stream = UnixStream::connect(slate_proto::socket_path())?;
        let env = Envelope {
            id: "attach".into(),
            request: Request::UiAttach {
                task_id: task_id.into(),
            },
        };
        stream.write_all(serde_json::to_string(&env)?.as_bytes())?;
        stream.write_all(b"\n")?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut writer = stream.try_clone()?;
        let handle = std::thread::spawn(move || {
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let Ok(ev) = serde_json::from_str::<Event>(&line) else {
                    continue;
                };
                if let Event::ApprovalNeeded {
                    approval_id,
                    tool_name,
                    summary,
                    tier,
                    reason,
                } = ev
                {
                    let (allow, remember) = ask_user(&tool_name, &summary, tier.as_str(), &reason);
                    let ans = Envelope {
                        id: format!("ans-{approval_id}"),
                        request: Request::ApprovalAnswer {
                            approval_id,
                            allow,
                            remember,
                        },
                    };
                    if let Ok(s) = serde_json::to_string(&ans) {
                        let _ = writer.write_all(s.as_bytes());
                        let _ = writer.write_all(b"\n");
                    }
                }
            }
        });
        Ok(Self {
            stream,
            handle: Some(handle),
        })
    }

    pub fn stop(mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn ask_user(tool_name: &str, summary: &str, tier: &str, reason: &str) -> (bool, bool) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out);
    let _ = writeln!(out, "{} {} {}", yellow("⏸"), bold(tool_name), cyan(summary));
    let _ = writeln!(
        out,
        "  {} {}   {} once  {} always this task  {} deny",
        dim(tier),
        dim(reason),
        bold("[y]"),
        bold("[a]"),
        bold("[n]")
    );
    let _ = write!(out, "  {} ", cyan("›"));
    let _ = out.flush();
    drop(out);
    // Stray keystrokes may be waiting (an agent driving the desktop can type into this very
    // terminal); the answer must be what the human types from here on.
    let _ = nix::sys::termios::tcflush(std::io::stdin(), nix::sys::termios::FlushArg::TCIFLUSH);
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return (false, false);
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => (true, false),
        "a" | "always" => (true, true),
        other => {
            println!(
                "  {} {}",
                red("denied"),
                dim(&format!("(got {other:?}; answer y, a or n)"))
            );
            (false, false)
        }
    }
}

/// Path of a sibling binary (same directory as this executable), else the bare name.
pub fn sibling_bin(name: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(name);
            if p.exists() {
                return p;
            }
        }
    }
    PathBuf::from(name)
}
