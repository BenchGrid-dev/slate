//! Append-only JSONL audit log at `<state>/audit.jsonl`.

use anyhow::{Context, Result};
use slate_proto::AuditEntry;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Audit {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Audit {
    pub fn open(dir: &std::path::Path) -> Result<Self> {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self {
            path: dir.join("audit.jsonl"),
            lock: Mutex::new(()),
        })
    }

    pub fn append(&self, entry: &AuditEntry) -> Result<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        serde_json::to_writer(&mut f, entry)?;
        f.write_all(b"\n")?;
        Ok(())
    }

    pub fn tail(&self, n: usize) -> Result<Vec<AuditEntry>> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(f) = fs::File::open(&self.path) else {
            return Ok(vec![]);
        };
        let lines: Vec<String> = BufReader::new(f).lines().map_while(Result::ok).collect();
        let start = lines.len().saturating_sub(n);
        Ok(lines[start..]
            .iter()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect())
    }
}
