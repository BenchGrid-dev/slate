//! The long-lived owner of the agent seat.
//!
//! Toolkits only accept input from seats that existed when the app started
//! (measured: Firefox/GTK ignore seats added later), so one daemon per session
//! creates the seat before any app runs and keeps it for the session's life.
//! The MCP server and the CLI send tool calls here as JSON lines:
//! `{"name": "...", "args": {...}}` -> the MCP result object.

use crate::mcp;
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
    let mut desktop = Desktop::connect()?;
    desktop.settle()?;
    let listener =
        UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
    slate_proto::log!(
        "slate-desktop daemon: agent seat ready, listening on {}",
        path.display()
    );
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
