//! Task records, persisted as JSON so undo works across daemon restarts.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use slate_proto::{Backend, TaskSummary};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub task_id: String,
    pub started: u64,
    pub ended: Option<u64>,
    pub backend: Backend,
    pub prompt: String,
    pub cwd: PathBuf,
    /// Snapshot path, if one was taken for this task.
    pub snapshot: Option<PathBuf>,
    pub tool_calls: u32,
    /// Absolute paths the task touched (from tool inputs), for undo scoping.
    pub touched: Vec<PathBuf>,
    /// Tools the user said "always allow" for this task.
    #[serde(default)]
    pub remembered: HashSet<String>,
    #[serde(default)]
    pub undone: bool,
    #[serde(default)]
    pub auto_approve: bool,
    /// No desktop notifications: the UI that started the task shows everything itself.
    #[serde(default)]
    pub quiet: bool,
}

impl Task {
    pub fn summary(&self) -> TaskSummary {
        TaskSummary {
            task_id: self.task_id.clone(),
            started: self.started,
            ended: self.ended,
            backend: self.backend.clone(),
            prompt: self.prompt.clone(),
            cwd: self.cwd.clone(),
            snapshot: self.snapshot.as_ref().map(|p| p.display().to_string()),
            tool_calls: self.tool_calls,
        }
    }
}

#[derive(Default)]
pub struct TaskStore {
    path: PathBuf,
    tasks: HashMap<String, Task>,
    order: Vec<String>,
}

impl TaskStore {
    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join("tasks.json");
        let mut s = Self {
            path,
            ..Default::default()
        };
        if let Ok(text) = std::fs::read_to_string(&s.path) {
            if let Ok(list) = serde_json::from_str::<Vec<Task>>(&text) {
                for t in list {
                    s.order.push(t.task_id.clone());
                    s.tasks.insert(t.task_id.clone(), t);
                }
            }
        }
        Ok(s)
    }

    fn persist(&self) -> Result<()> {
        let list: Vec<&Task> = self
            .order
            .iter()
            .filter_map(|id| self.tasks.get(id))
            .collect();
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&list)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn insert(&mut self, task: Task) -> Result<()> {
        self.order.push(task.task_id.clone());
        self.tasks.insert(task.task_id.clone(), task);
        // Keep the store bounded.
        while self.order.len() > 500 {
            let old = self.order.remove(0);
            self.tasks.remove(&old);
        }
        self.persist()
    }

    pub fn get(&self, id: &str) -> Option<&Task> {
        self.tasks.get(id)
    }

    pub fn update<F: FnOnce(&mut Task)>(&mut self, id: &str, f: F) -> Result<bool> {
        let Some(t) = self.tasks.get_mut(id) else {
            return Ok(false);
        };
        f(t);
        self.persist()?;
        Ok(true)
    }

    /// The most recent task that has a snapshot and has not been undone.
    pub fn last_undoable(&self) -> Option<&Task> {
        self.order
            .iter()
            .rev()
            .filter_map(|id| self.tasks.get(id))
            .find(|t| t.snapshot.as_ref().map(|p| p.exists()).unwrap_or(false) && !t.undone)
    }

    pub fn recent(&self, n: usize) -> Vec<TaskSummary> {
        self.order
            .iter()
            .rev()
            .take(n)
            .filter_map(|id| self.tasks.get(id))
            .map(Task::summary)
            .collect()
    }
}

pub fn new_task_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{:x}-{:x}-{:x}",
        slate_proto::now_millis(),
        std::process::id(),
        n
    )
}
