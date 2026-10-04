//! The long-lived owner of the agent seat.
//!
//! Toolkits only accept input from seats that existed when the app started
//! (measured: Firefox/GTK ignore seats added later), so one daemon per session
//! creates the seat before any app runs and keeps it for the session's life.
//! The MCP server and the CLI send tool calls here as JSON lines:
//! `{"name": "...", "args": {...}}` -> the MCP result object.

use crate::mcp;
use crate::sway;
use crate::wayland::Desktop;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};

pub fn run() -> Result<()> {
    let path = slate_proto::desktop_socket_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            anyhow::bail!(
                "another slate-desktop daemon is listening on {}",
                path.display()
            );
        }
        std::fs::remove_file(&path)?;
    }
    // The background screen must not displace the real ones, and the person's focus
    // must start on a real screen (sway would otherwise focus the first output, which
    // is the headless one).
    if sway::available() {
        if let Err(e) = sway::fix_layout() {
            slate_proto::log!("slate-desktop daemon: layout: {e:#}");
        }
    }
    let mut desktop = Desktop::connect()?;
    desktop.settle()?;
    if sway::available() {
        mcp::park_agent_pointer(&mut desktop);
        // Once more after the seat has settled: right after a restart the first
        // motion can land before sway has placed the new seat's pointers.
        std::thread::sleep(std::time::Duration::from_secs(1));
        mcp::park_agent_pointer(&mut desktop);
    }
    let listener =
        UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
    slate_proto::log!(
        "slate-desktop daemon: agent seat ready, listening on {}",
        path.display()
    );
    std::thread::spawn(|| {
        let mut n = 0u32;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            mcp::takeover_tick();
            n += 1;
            // Every 5 s: a hotplugged or reconfigured output may have been auto-placed
            // next to the background screen.
            if n % 10 == 0 && sway::available() {
                let _ = sway::fix_layout();
            }
        }
    });
    // Windows the agent's engine starts from the shell would open on the person's
    // screen; the watcher moves them to the background as they appear.
    std::thread::spawn(|| loop {
        if sway::available() {
            let r = sway::watch_windows(|change, con| {
                if change == "new" {
                    let id = con.get("id").and_then(Value::as_i64);
                    let pid = con.get("pid").and_then(Value::as_u64).map(|p| p as u32);
                    let app = con.get("app_id").and_then(Value::as_str).unwrap_or("");
                    if let Some(id) = id {
                        mcp::place_new_window(id, pid, app);
                    }
                }
            });
            if let Err(e) = r {
                slate_proto::log!("slate-desktop daemon: window watcher: {e:#}");
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    });
    for conn in listener.incoming() {
        let Ok(stream) = conn else { continue };
        if let Err(e) = handle(&mut desktop, stream) {
            slate_proto::log!("slate-desktop daemon: {e:#}");
        }
    }
    Ok(())
}

fn handle(desktop: &mut Desktop, stream: UnixStream) -> Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => {
                let name = req.get("name").and_then(Value::as_str).unwrap_or("");
                let args = req.get("args").cloned().unwrap_or(Value::Null);
                mcp::call(desktop, name, &args)
            }
            Err(e) => {
                json!({"content": [{"type": "text", "text": format!("bad request: {e}")}], "isError": true})
            }
        };
        writer.write_all(serde_json::to_string(&reply)?.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

/// A connection to the daemon, if one is running.
pub struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    pub fn connect() -> Option<Self> {
        let stream = UnixStream::connect(slate_proto::desktop_socket_path()).ok()?;
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(120)))
            .ok()?;
        let reader = BufReader::new(stream.try_clone().ok()?);
        Some(Self { stream, reader })
    }

    pub fn call(&mut self, name: &str, args: &Value) -> Result<Value> {
        let req = json!({"name": name, "args": args});
        self.stream
            .write_all(serde_json::to_string(&req)?.as_bytes())?;
        self.stream.write_all(b"\n")?;
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            anyhow::bail!("desktop daemon closed the connection");
        }
        Ok(serde_json::from_str(&line)?)
    }
}
