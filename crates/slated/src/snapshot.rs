//! Filesystem snapshots and undo on btrfs.
//!
//! The user's home must be its own btrfs subvolume (or `snapshot_root` must
//! point at one). Snapshots are read-only subvolumes under
//! `<state>/snapshots/<task_id>`. Creating a snapshot needs the privileges
//! the system grants for `btrfs subvolume snapshot`; slated tries plainly,
//! then `sudo -n` (non-interactive) if the system is configured for it, and
//! otherwise reports that snapshots are unavailable.
//!
//! Undo restores files reported by `btrfs subvolume find-new` since the
//! snapshot's generation, and recreates files deleted within the task's
//! touched directories. It never touches Slate's own state or agent caches.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Snapshotter {
    /// The subvolume we snapshot (normally $HOME).
    pub root: PathBuf,
    /// Where snapshots live.
    pub dir: PathBuf,
    pub use_sudo: bool,
}

#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub path: PathBuf,
    pub generation: u64,
}

#[derive(Debug, Default, Clone)]
pub struct UndoPlan {
    pub restore: Vec<PathBuf>,
    pub delete: Vec<PathBuf>,
    pub recreate: Vec<PathBuf>,
}

/// Relative paths under the root that undo must never touch.
const PROTECTED: &[&str] = &[
    ".local/state/slate",
    ".cache",
    ".claude",
    ".codex",
    ".local/share/slate",
    ".npm",
    ".cargo/registry",
    ".rustup",
];

impl Snapshotter {
    /// Detect whether snapshots can work for `root`.
    pub fn detect(root: PathBuf, dir: PathBuf) -> Result<Self> {
        if !is_subvolume(&root) {
            bail!("{} is not a btrfs subvolume", root.display());
        }
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            root,
            dir,
            use_sudo: false,
        })
    }

    fn btrfs(&self) -> Command {
        if self.use_sudo {
            let mut c = Command::new("sudo");
            c.arg("-n").arg("btrfs");
            c
        } else {
            Command::new("btrfs")
        }
    }

    pub fn create(&mut self, task_id: &str) -> Result<SnapshotInfo> {
        let path = self.dir.join(task_id);
        let generation = self.current_generation()?;
        let out = self
            .btrfs()
            .args(["subvolume", "snapshot", "-r"])
            .arg(&self.root)
            .arg(&path)
            .output()
            .context("running btrfs")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if !self.use_sudo && err.contains("not permitted") {
                // Try once with sudo -n; if the system allows it, remember that.
                self.use_sudo = true;
                let out2 = self
                    .btrfs()
                    .args(["subvolume", "snapshot", "-r"])
                    .arg(&self.root)
                    .arg(&path)
                    .output()?;
                if out2.status.success() {
                    return Ok(SnapshotInfo { path, generation });
                }
                self.use_sudo = false;
            }
            bail!("btrfs snapshot failed: {}", err.trim());
        }
        Ok(SnapshotInfo { path, generation })
    }

    /// Current transid of the root subvolume, via `find-new <root> <huge>`.
    pub fn current_generation(&self) -> Result<u64> {
        let out = Command::new("btrfs")
            .args(["subvolume", "find-new"])
            .arg(&self.root)
            .arg("18446744073709551615")
            .output()?;
        let text = String::from_utf8_lossy(&out.stdout);
        parse_transid_marker(&text)
            .ok_or_else(|| anyhow!("could not read generation: {}", text.trim()))
    }

    /// Files (relative to root) modified or created since `generation`.
    pub fn changed_since(&self, generation: u64) -> Result<Vec<PathBuf>> {
        let out = Command::new("btrfs")
            .args(["subvolume", "find-new"])
            .arg(&self.root)
            .arg(generation.to_string())
            .output()?;
        if !out.status.success() {
            bail!(
                "find-new failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut set = BTreeSet::new();
        for line in text.lines() {
            if let Some(p) = parse_find_new_line(line) {
                set.insert(PathBuf::from(p));
            }
        }
        Ok(set.into_iter().collect())
    }

    /// Work out what undo would do. `scope` is a list of absolute directories
    /// the task worked in; deletions are only detected inside them.
    pub fn plan(&self, snap: &SnapshotInfo, scope: &[PathBuf]) -> Result<UndoPlan> {
        let mut plan = UndoPlan::default();
        for rel in self.changed_since(snap.generation)? {
            if is_protected(&rel) {
                continue;
            }
            let in_snap = snap.path.join(&rel);
            if in_snap.exists() {
                plan.restore.push(rel);
            } else {
                plan.delete.push(rel);
            }
        }
        // Deletions: walk the snapshot side of each scope dir, bounded.
        let mut budget = 20_000usize;
        for dir in scope {
            let Ok(rel_dir) = dir.strip_prefix(&self.root) else {
                continue;
            };
            if is_protected(rel_dir) {
                continue;
            }
            let snap_dir = snap.path.join(rel_dir);
            if !snap_dir.is_dir() {
                continue;
            }
            walk(&snap_dir, &mut |p| {
                if budget == 0 {
                    return false;
                }
                budget -= 1;
                if let Ok(rel) = p.strip_prefix(&snap.path) {
                    if is_protected(rel) {
                        return false;
                    }
                    let live = self.root.join(rel);
                    if !live.exists() && !live.is_symlink() {
                        plan.recreate.push(rel.to_path_buf());
                        return false; // no need to descend into a missing dir
                    }
                }
                true
            });
        }
        Ok(plan)
    }

    pub fn apply(
        &self,
        snap: &SnapshotInfo,
        plan: &UndoPlan,
    ) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
        let mut restored = vec![];
        let mut deleted = vec![];
        for rel in plan.restore.iter().chain(plan.recreate.iter()) {
            let from = snap.path.join(rel);
            let to = self.root.join(rel);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let status = Command::new("cp")
                .args(["-a", "--reflink=auto"])
                .arg(&from)
                .arg(&to)
                .status()?;
            if status.success() {
                restored.push(rel.clone());
            }
        }
        for rel in &plan.delete {
            let to = self.root.join(rel);
            let ok = if to.is_dir() && !to.is_symlink() {
                std::fs::remove_dir_all(&to).is_ok()
            } else {
                std::fs::remove_file(&to).is_ok()
            };
            if ok {
                deleted.push(rel.clone());
            }
        }
        Ok((restored, deleted))
    }
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path) -> bool) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let descend = f(&p);
        if descend && p.is_dir() && !p.is_symlink() {
            walk(&p, f);
        }
    }
}

fn is_protected(rel: &Path) -> bool {
    let s = rel.to_string_lossy();
    PROTECTED
        .iter()
        .any(|p| s == *p || s.starts_with(&format!("{p}/")))
}

pub fn is_subvolume(p: &Path) -> bool {
    // A btrfs subvolume root always has inode 256.
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p)
        .map(|m| m.ino() == 256)
        .unwrap_or(false)
        && Command::new("btrfs")
            .args(["subvolume", "show"])
            .arg(p)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

fn parse_transid_marker(text: &str) -> Option<u64> {
    // Last line: "transid marker was 1234"
    text.lines().rev().find_map(|l| {
        l.trim()
            .strip_prefix("transid marker was ")
            .and_then(|n| n.trim().parse().ok())
    })
}

fn parse_find_new_line(line: &str) -> Option<&str> {
    // "inode 261 file offset 0 len 12 disk start 0 offset 0 gen 47 flags INLINE some/path"
    if !line.starts_with("inode ") {
        return None;
    }
    let idx = line.find(" flags ")?;
    let rest = &line[idx + 7..];
    let (_flags, path) = rest.split_once(' ')?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_find_new() {
        assert_eq!(
            parse_find_new_line("inode 261 file offset 0 len 12 disk start 0 offset 0 gen 47 flags INLINE src/a b.rs"),
            Some("src/a b.rs")
        );
        assert_eq!(parse_find_new_line("transid marker was 48"), None);
        assert_eq!(
            parse_transid_marker("inode ...\ntransid marker was 48\n"),
            Some(48)
        );
    }

    #[test]
    fn protected_paths() {
        assert!(is_protected(Path::new(".claude/settings.json")));
        assert!(is_protected(Path::new(".local/state/slate/audit.jsonl")));
        assert!(!is_protected(Path::new("src/main.rs")));
        assert!(!is_protected(Path::new(".claude2/x")));
    }
}
