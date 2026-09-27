//! Minimal request/reply client for slated.

use anyhow::{bail, Context, Result};
use slate_proto::{Envelope, Reply, ReplyEnvelope, Request};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl Client {
    pub fn connect() -> Result<Self> {
        let path = slate_proto::socket_path();
        let stream = UnixStream::connect(&path).with_context(|| {
            format!(
                "connecting to slated at {} (is it running?)",
                path.display()
            )
        })?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            next_id: 1,
        })
    }

    pub fn set_timeout(&mut self, d: Option<Duration>) -> Result<()> {
        self.stream.set_read_timeout(d)?;
        Ok(())
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
            let n = self.reader.read_line(&mut line)?;
            if n == 0 {
                bail!("slated closed the connection");
            }
            // Skip unsolicited events on this connection (none expected unless attached).
            if line.contains("\"event\"") && !line.contains("\"id\"") {
                continue;
            }
            let r: ReplyEnvelope =
                serde_json::from_str(&line).with_context(|| format!("bad reply: {line}"))?;
            if r.id == id {
                return Ok(r.reply);
            }
        }
    }
}
