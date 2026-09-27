//! User memories: things the user asked to remember, as append-only JSONL with
//! tombstones. Small on purpose; this is the "记一下" of the OS, not a vector DB.

use anyhow::Result;
use slate_proto::{now_millis, Memory};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Line {
    Add(Memory),
    Forget { id: String },
}

pub struct MemoryStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl MemoryStore {
    pub fn open(dir: &std::path::Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            path: dir.join("memory.jsonl"),
            lock: Mutex::new(()),
        })
    }

    fn append(&self, line: &Line) -> Result<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        serde_json::to_writer(&mut f, line)?;
        f.write_all(b"\n")?;
        Ok(())
    }

    /// Store a memory. An exact duplicate (ignoring case and surrounding whitespace)
    /// returns the existing memory instead of adding another.
    pub fn add(&self, text: String, task_id: Option<String>) -> Result<Memory> {
        let norm = text.trim().to_lowercase();
        if let Some(existing) = self
            .all()?
            .into_iter()
            .find(|m| m.text.to_lowercase() == norm)
        {
            return Ok(existing);
        }
        let m = Memory {
            id: format!("{:x}", now_millis()),
            ts: now_millis(),
            text: text.trim().to_string(),
            task_id,
        };
        self.append(&Line::Add(m.clone()))?;
        Ok(m)
    }

    pub fn forget(&self, id: &str) -> Result<bool> {
        let exists = self.all()?.iter().any(|m| m.id == id);
        if exists {
            self.append(&Line::Forget { id: id.to_string() })?;
        }
        Ok(exists)
    }

    /// All live memories, oldest first.
    pub fn all(&self) -> Result<Vec<Memory>> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(f) = std::fs::File::open(&self.path) else {
            return Ok(vec![]);
        };
        let mut out: Vec<Memory> = vec![];
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            match serde_json::from_str::<Line>(&line) {
                Ok(Line::Add(m)) => out.push(m),
                Ok(Line::Forget { id }) => out.retain(|m| m.id != id),
                Err(_) => {}
            }
        }
        Ok(out)
    }

    pub fn list(&self, n: usize, query: Option<&str>) -> Result<Vec<Memory>> {
        let mut all = self.all()?;
        if let Some(q) = query.map(|q| q.to_lowercase()).filter(|q| !q.is_empty()) {
            all.retain(|m| m.text.to_lowercase().contains(&q));
        }
        let start = all.len().saturating_sub(n);
        Ok(all[start..].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_list_forget() {
        let dir = tempfile::tempdir().unwrap();
        let s = MemoryStore::open(dir.path()).unwrap();
        let a = s.add("prefers dark mode".into(), None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        s.add("meeting with Alice on Friday".into(), Some("t1".into()))
            .unwrap();
        assert_eq!(s.list(10, None).unwrap().len(), 2);
        assert_eq!(s.list(10, Some("alice")).unwrap().len(), 1);
        assert!(s.forget(&a.id).unwrap());
        assert_eq!(s.list(10, None).unwrap().len(), 1);
        assert!(!s.forget("nope").unwrap());
    }

    #[test]
    fn duplicates_collapse() {
        let dir = tempfile::tempdir().unwrap();
        let s = MemoryStore::open(dir.path()).unwrap();
        let a = s.add("Likes tea".into(), None).unwrap();
        let b = s.add("  likes tea ".into(), None).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(s.list(10, None).unwrap().len(), 1);
    }
}
