//! Filesystem snapshots and undo on btrfs, without privileges.
//!
//! The snapshot root (normally $HOME, or `SLATE_SNAPSHOT_ROOT`) must be a btrfs
//! subvolume the user owns. An unprivileged user can create read-only snapshots
//! of their own subvolume and delete them again; that is all we need.
//! `find-new` and `subvolume show` need CAP_SYS_ADMIN, so change detection is
//! done by diffing the snapshot against the live tree inside the directories
//! the task worked in. Slate's own state and agent caches are never touched.

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Snapshotter {
    /// The subvolume we snapshot.
    pub root: PathBuf,
    /// Where snapshots live (same filesystem, outside `root` if possible).
    pub dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub path: PathBuf,
}

#[derive(Debug, Default, Clone)]
pub struct UndoPlan {
    /// Files that exist in both but differ: copy the snapshot's version back.
    pub restore: Vec<PathBuf>,
    /// Files created since the snapshot: remove.
    pub delete: Vec<PathBuf>,
    /// Files deleted since the snapshot: copy back.
    pub recreate: Vec<PathBuf>,
    /// Scope directories that were skipped or truncated, with why.
    pub notes: Vec<String>,
}

/// Relative paths under the root that undo must never touch.
const PROTECTED: &[&str] = &[
    ".local/state/slate",
    ".local/share/slate",
    ".cache",
    ".claude",
    ".codex",
    ".npm",
    ".cargo/registry",
    ".cargo/git",
    ".rustup",
    ".snapshots",
];

/// Maximum directory entries examined per undo.
const WALK_BUDGET: usize = 200_000;

const BTRFS_SUPER_MAGIC: i64 = 0x9123_683E;

impl Snapshotter {
    pub fn detect(root: PathBuf, dir: PathBuf) -> Result<Self> {
        if !is_subvolume(&root) {
            bail!("{} is not a btrfs subvolume", root.display());
        }
        let uid = unsafe { libc::getuid() };
        let owner = std::fs::metadata(&root)?.uid();
        if owner != uid {
            bail!("{} is owned by uid {owner}, not by this user ({uid}); unprivileged snapshots need a user-owned subvolume", root.display());
        }
        std::fs::create_dir_all(&dir)?;
        Ok(Self { root, dir })
    }

    pub fn create(&self, task_id: &str) -> Result<SnapshotInfo> {
        let path = self.dir.join(task_id);
        let out = Command::new("btrfs")
            .args(["subvolume", "snapshot", "-r"])
            .arg(&self.root)
            .arg(&path)
            .output()
            .context("running btrfs")?;
        if !out.status.success() {
            bail!(
                "btrfs snapshot failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(SnapshotInfo { path })
    }

    /// Remove the oldest snapshots so at most `keep` remain.
    pub fn prune(&self, keep: usize) -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return vec![];
        };
        let mut snaps: Vec<(std::time::SystemTime, PathBuf)> = rd
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let t = e.metadata().and_then(|m| m.modified()).ok()?;
                Some((t, p))
            })
            .collect();
        snaps.sort();
        let mut removed = vec![];
        while snaps.len() > keep {
            let (_, p) = snaps.remove(0);
            if self.delete(&p).is_ok() {
                removed.push(p);
            }
        }
        removed
    }

    pub fn delete(&self, snap: &Path) -> Result<()> {
        let out = Command::new("btrfs")
            .args(["subvolume", "delete"])
            .arg(snap)
            .output()?;
        if !out.status.success() {
            bail!(
                "btrfs delete failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    /// Diff the snapshot against the live tree inside `scope` (absolute dirs).
    pub fn plan(&self, snap: &SnapshotInfo, scope: &[PathBuf]) -> Result<UndoPlan> {
        let mut plan = UndoPlan::default();
        let mut budget = WALK_BUDGET;
        let mut seen_scopes: Vec<PathBuf> = vec![];
        for dir in scope {
            let Ok(rel_dir) = dir.strip_prefix(&self.root) else {
                plan.notes
                    .push(format!("{} is outside the snapshot root", dir.display()));
                continue;
            };
            if is_protected(rel_dir) {
                continue;
            }
            // Skip scopes nested in one we already covered.
            if seen_scopes.iter().any(|s| rel_dir.starts_with(s)) {
                continue;
            }
            seen_scopes.push(rel_dir.to_path_buf());

            let live = self.root.join(rel_dir);
            let snapd = snap.path.join(rel_dir);
            let mut live_map = BTreeMap::new();
            let mut snap_map = BTreeMap::new();
            let truncated = !collect(&live, &live, &mut live_map, &mut budget)
                | !collect(&snapd, &snapd, &mut snap_map, &mut budget);
            if truncated {
                plan.notes.push(format!(
                    "{} was too large to diff completely",
                    dir.display()
                ));
            }
            for (rel, live_e) in &live_map {
                let full = rel_dir.join(rel);
                if is_protected(&full) {
                    continue;
                }
                match snap_map.get(rel) {
                    None => plan.delete.push(full),
                    Some(snap_e) => {
                        if live_e.differs(snap_e) {
                            plan.restore.push(full);
                        }
                    }
                }
            }
            for rel in snap_map.keys() {
                let full = rel_dir.join(rel);
                if !live_map.contains_key(rel) && !is_protected(&full) {
                    plan.recreate.push(full);
                }
            }
        }
        // Deleting a directory deletes its children; drop redundant entries.
        prune_nested(&mut plan.delete);
        prune_nested(&mut plan.recreate);
        Ok(plan)
    }

    pub fn apply(
        &self,
        snap: &SnapshotInfo,
        plan: &UndoPlan,
    ) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
        let mut restored = vec![];
        let mut deleted = vec![];
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
        for rel in plan.restore.iter().chain(plan.recreate.iter()) {
            let from = snap.path.join(rel);
            let to = self.root.join(rel);
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if to.exists() || to.is_symlink() {
                if to.is_dir() && !to.is_symlink() {
                    let _ = std::fs::remove_dir_all(&to);
                } else {
                    let _ = std::fs::remove_file(&to);
                }
            }
            let mut cp = Command::new("cp");
            cp.arg("-a");
            if cfg!(target_os = "linux") {
                cp.arg("--reflink=auto");
            }
            let status = cp.arg(&from).arg(&to).status()?;
            if status.success() {
                restored.push(rel.clone());
            }
        }
        Ok((restored, deleted))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    kind: Kind,
    size: u64,
    mtime: i64,
    mtime_nsec: i64,
    link: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Dir,
    Symlink,
    Other,
}

impl Entry {
    fn differs(&self, other: &Entry) -> bool {
        if self.kind != other.kind {
            return true;
        }
        match self.kind {
            Kind::File => {
                self.size != other.size
                    || self.mtime != other.mtime
                    || self.mtime_nsec != other.mtime_nsec
            }
            Kind::Symlink => self.link != other.link,
            Kind::Dir | Kind::Other => false,
        }
    }
}

/// Walk `dir`, recording entries relative to `base`. Returns false if the budget ran out.
fn collect(
    base: &Path,
    dir: &Path,
    out: &mut BTreeMap<PathBuf, Entry>,
    budget: &mut usize,
) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return true;
    };
    for e in rd.flatten() {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        let p = e.path();
        let Ok(md) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        let Ok(rel) = p.strip_prefix(base) else {
            continue;
        };
        let ft = md.file_type();
        let kind = if ft.is_symlink() {
            Kind::Symlink
        } else if ft.is_dir() {
            Kind::Dir
        } else if ft.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        let link = if kind == Kind::Symlink {
            std::fs::read_link(&p).ok()
        } else {
            None
        };
        out.insert(
            rel.to_path_buf(),
            Entry {
                kind,
                size: md.len(),
                mtime: md.mtime(),
                mtime_nsec: md.mtime_nsec(),
                link,
            },
        );
        if kind == Kind::Dir && !collect(base, &p, out, budget) {
            return false;
        }
    }
    true
}

fn prune_nested(paths: &mut Vec<PathBuf>) {
    paths.sort();
    let mut out: Vec<PathBuf> = vec![];
    for p in paths.drain(..) {
        if out.last().map(|last| p.starts_with(last)).unwrap_or(false) {
            continue;
        }
        out.push(p);
    }
    *paths = out;
}

fn is_protected(rel: &Path) -> bool {
    let s = rel.to_string_lossy();
    PROTECTED
        .iter()
        .any(|p| s == *p || s.starts_with(&format!("{p}/")))
}

pub fn is_subvolume(p: &Path) -> bool {
    // A btrfs subvolume root always has inode 256, on a btrfs filesystem.
    let Ok(md) = std::fs::metadata(p) else {
        return false;
    };
    if md.ino() != 256 {
        return false;
    }
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(c.as_ptr(), &mut st) };
    rc == 0 && (st.f_type as i64) == BTRFS_SUPER_MAGIC
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(p: &Path, content: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    /// Simulate a snapshot with a plain directory copy; the diff logic is fs-agnostic.
    #[test]
    fn plan_detects_changes_within_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let snap = tmp.path().join("snap");
        touch(&root.join("proj/keep.txt"), "same");
        touch(&root.join("proj/changed.txt"), "old");
        touch(&root.join("proj/gone.txt"), "bye");
        touch(&root.join("other/x.txt"), "out of scope");
        // "snapshot"
        let st = Command::new("cp")
            .args(["-a"])
            .arg(&root)
            .arg(&snap)
            .status()
            .unwrap();
        assert!(st.success());
        // mutate live
        std::thread::sleep(std::time::Duration::from_millis(20));
        touch(&root.join("proj/changed.txt"), "new content");
        fs::remove_file(root.join("proj/gone.txt")).unwrap();
        touch(&root.join("proj/new/deep.txt"), "created");
        touch(&root.join("other/x.txt"), "also changed but out of scope");

        let s = Snapshotter {
            root: root.clone(),
            dir: tmp.path().join("snaps"),
        };
        let plan = s
            .plan(&SnapshotInfo { path: snap.clone() }, &[root.join("proj")])
            .unwrap();
        assert_eq!(plan.restore, vec![PathBuf::from("proj/changed.txt")]);
        assert_eq!(plan.delete, vec![PathBuf::from("proj/new")]);
        assert_eq!(plan.recreate, vec![PathBuf::from("proj/gone.txt")]);

        let (restored, deleted) = s.apply(&SnapshotInfo { path: snap }, &plan).unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(deleted.len(), 1);
        assert_eq!(
            fs::read_to_string(root.join("proj/changed.txt")).unwrap(),
            "old"
        );
        assert_eq!(
            fs::read_to_string(root.join("proj/gone.txt")).unwrap(),
            "bye"
        );
        assert!(!root.join("proj/new").exists());
        assert_eq!(
            fs::read_to_string(root.join("other/x.txt")).unwrap(),
            "also changed but out of scope"
        );
    }

    #[test]
    fn protected_paths() {
        assert!(is_protected(Path::new(".claude/settings.json")));
        assert!(is_protected(Path::new(".local/state/slate/audit.jsonl")));
        assert!(!is_protected(Path::new("src/main.rs")));
        assert!(!is_protected(Path::new(".claude2/x")));
    }

    #[test]
    fn prune() {
        let mut v = vec![
            PathBuf::from("a/b/c"),
            PathBuf::from("a/b"),
            PathBuf::from("x"),
        ];
        prune_nested(&mut v);
        assert_eq!(v, vec![PathBuf::from("a/b"), PathBuf::from("x")]);
    }
}
