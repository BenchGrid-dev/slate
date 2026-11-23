//! Filesystem snapshots and undo on btrfs, without privileges.
//!
//! The snapshot root (normally $HOME, or `SLATE_SNAPSHOT_ROOT`) must be a btrfs
//! subvolume the user owns. An unprivileged user can create read-only snapshots
//! of their own subvolume. They can delete them again only where the filesystem
//! is mounted with `user_subvol_rm_allowed`, and only once the snapshot is
//! writable, which its owner may make it (docs/decisions/0013).
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

/// The mount option without which the kernel lets only root delete a subvolume.
pub const REMOVAL_OPTION: &str = "user_subvol_rm_allowed";

/// What a round of pruning did.
#[derive(Debug, Default)]
pub struct Pruned {
    pub removed: Vec<PathBuf>,
    /// Why it stopped before it was done; the snapshots left stay for the next round.
    pub failed: Option<String>,
}

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

    /// Remove the oldest snapshots so at most `keep` remain. Stops at the first one
    /// that cannot be removed: what stops one stops them all.
    pub fn prune(&self, keep: usize) -> Pruned {
        let mut done = Pruned::default();
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return done;
        };
        let names = rd
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok());
        for name in beyond(names, keep) {
            let path = self.dir.join(name);
            match self.delete(&path) {
                Ok(()) => done.removed.push(path),
                Err(e) => {
                    done.failed = Some(format!("{e:#}"));
                    break;
                }
            }
        }
        done
    }

    pub fn delete(&self, snap: &Path) -> Result<()> {
        if let Some(why) = self.removal_blocked() {
            bail!(why);
        }
        // A user may delete only a subvolume they could write to, and a snapshot is
        // read-only until its owner says otherwise.
        set_read_only(snap, false)?;
        let out = Command::new("btrfs")
            .args(["subvolume", "delete"])
            .arg(snap)
            .output()
            .context("running btrfs")?;
        if !out.status.success() {
            // Still there: as it was, so nothing writes into what undo reads.
            let _ = set_read_only(snap, true);
            bail!(
                "btrfs delete failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    /// Why old snapshots cannot be removed here, when they cannot: the filesystem is
    /// mounted without the option that lets a user delete a subvolume.
    pub fn removal_blocked(&self) -> Option<String> {
        if unsafe { libc::geteuid() } == 0 {
            return None;
        }
        // Unreadable or unknown: try, and let btrfs say.
        let mounts = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
        let dir = std::fs::canonicalize(&self.dir).unwrap_or_else(|_| self.dir.clone());
        match mounted_with(&mounts, &dir, REMOVAL_OPTION)? {
            (_, true) => None,
            (mount, false) => Some(format!(
                "{} is mounted without {REMOVAL_OPTION}, so only root can delete a snapshot \
                 (SlateOS sets the option when it starts; elsewhere add it to the mount's options)",
                mount.display()
            )),
        }
    }

    /// Whether undo could restore `path` (absolute, normalized): inside the root and
    /// not among what undo never touches.
    pub fn covers(&self, path: &Path) -> bool {
        path.strip_prefix(&self.root)
            .map(|rel| !is_protected(rel))
            .unwrap_or(false)
    }

    /// Diff the snapshot against the live tree inside `scope` (absolute dirs).
    pub fn plan(&self, snap: &SnapshotInfo, scope: &[PathBuf]) -> Result<UndoPlan> {
        let mut plan = UndoPlan::default();
        let mut budget = WALK_BUDGET;
        let mut seen_scopes: Vec<PathBuf> = vec![];
        // Each scope relative to the root. A task that worked in a directory above the
        // root (its cwd was the home directory, the root a subvolume inside it) may
        // have changed anything under the root: compare all of it.
        let mut rel_scopes: Vec<(PathBuf, &PathBuf)> = vec![];
        for dir in scope {
            match dir.strip_prefix(&self.root) {
                Ok(rel) => rel_scopes.push((rel.to_path_buf(), dir)),
                Err(_) if self.root.starts_with(dir) => rel_scopes.push((PathBuf::new(), dir)),
                Err(_) => plan
                    .notes
                    .push(format!("{} is outside the snapshot root", dir.display())),
            }
        }
        // Outermost first, so the nested ones are skipped below.
        rel_scopes.sort_by_key(|(rel, _)| rel.components().count());
        for (rel_dir, dir) in &rel_scopes {
            let rel_dir = rel_dir.as_path();
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

/// Prune on a thread of its own: no tool call waits for old snapshots to go. One round
/// at a time; a call while one runs does nothing, and the next snapshot asks again.
pub fn prune_in_background(snap: &Snapshotter, keep: usize) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    static RUNNING: AtomicBool = AtomicBool::new(false);
    // What stopped the last round, to say it once and not with every task.
    static STOPPED_BY: Mutex<Option<String>> = Mutex::new(None);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let snap = snap.clone();
    std::thread::spawn(move || {
        let pruned = snap.prune(keep);
        for old in &pruned.removed {
            slate_proto::log!("slated: pruned snapshot {}", old.display());
        }
        let mut said = STOPPED_BY.lock().unwrap_or_else(|e| e.into_inner());
        if *said != pruned.failed {
            match &pruned.failed {
                Some(why) => slate_proto::log!("slated: old snapshots are not removed: {why}"),
                None => slate_proto::log!("slated: old snapshots are removed again"),
            }
            *said = pruned.failed;
        }
        drop(said);
        RUNNING.store(false, Ordering::SeqCst);
    });
}

/// When the snapshot of this name was taken, in milliseconds. slated names a snapshot
/// after its task, and a task's id begins with the time it started (`tasks::new_task_id`).
/// The directory's own times cannot say: a snapshot carries those of what it was taken of.
fn taken_at(name: &str) -> Option<u64> {
    let mut parts = name.split('-');
    let at = u64::from_str_radix(parts.next()?, 16).ok()?;
    (parts.count() == 2).then_some(at)
}

/// The snapshots to remove so that the `keep` newest remain, oldest first. A name that
/// is not a task's is not slated's to remove, and does not count.
fn beyond(names: impl Iterator<Item = String>, keep: usize) -> Vec<String> {
    let mut snaps: Vec<(u64, String)> = names
        .filter_map(|n| taken_at(&n).map(|at| (at, n)))
        .collect();
    snaps.sort();
    let extra = snaps.len().saturating_sub(keep);
    snaps.into_iter().take(extra).map(|(_, n)| n).collect()
}

fn set_read_only(snap: &Path, read_only: bool) -> Result<()> {
    let out = Command::new("btrfs")
        .args(["property", "set", "-ts"])
        .arg(snap)
        .args(["ro", if read_only { "true" } else { "false" }])
        .output()
        .context("running btrfs")?;
    if !out.status.success() {
        bail!(
            "btrfs property set failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// The mount that holds `path` (absolute, symlinks resolved) and whether it carries
/// `option`, from the text of `/proc/self/mountinfo`. None when no line holds the path.
fn mounted_with(mountinfo: &str, path: &Path, option: &str) -> Option<(PathBuf, bool)> {
    let mut found: Option<(PathBuf, bool)> = None;
    for line in mountinfo.lines() {
        // ID PARENT MAJ:MIN ROOT MOUNT-POINT OPTIONS [TAG…] - TYPE SOURCE SUPER-OPTIONS
        let Some((mount, fs)) = line.split_once(" - ") else {
            continue;
        };
        let mount: Vec<&str> = mount.split(' ').collect();
        let (Some(point), Some(options)) = (mount.get(4), mount.get(5)) else {
            continue;
        };
        let point = PathBuf::from(unescape(point));
        if !path.starts_with(&point) {
            continue;
        }
        // The deepest mount holds the path; of two on one point, the later lies on top.
        let deeper = found
            .as_ref()
            .map(|(p, _)| point.as_os_str().len() >= p.as_os_str().len())
            .unwrap_or(true);
        if deeper {
            let of_the_filesystem = fs.split(' ').nth(2).unwrap_or("");
            let has = options
                .split(',')
                .chain(of_the_filesystem.split(','))
                .any(|o| o == option);
            found = Some((point, has));
        }
    }
    found
}

/// mountinfo writes a space, tab, newline or backslash in a path as `\040` and so on.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = (bytes[i] == b'\\' && i + 3 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 4]).ok())
            .flatten()
            .and_then(|d| u8::from_str_radix(d, 8).ok());
        match octal {
            Some(byte) => {
                out.push(byte);
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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

    /// The task's directory contains the snapshot root (cwd = home, root = ~/work):
    /// everything under the root is in scope, and a scope inside it is not diffed twice.
    #[test]
    fn plan_covers_the_root_when_the_task_worked_above_it() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let root = home.join("work");
        let snap = tmp.path().join("snap");
        touch(&root.join("inbox/a.pdf"), "a");
        touch(&root.join("inbox/b.jpg"), "b");
        let st = Command::new("cp")
            .args(["-a"])
            .arg(&root)
            .arg(&snap)
            .status()
            .unwrap();
        assert!(st.success());
        // The files moved out of the root entirely (to ~/Documents).
        fs::remove_file(root.join("inbox/a.pdf")).unwrap();
        fs::remove_file(root.join("inbox/b.jpg")).unwrap();
        let s = Snapshotter {
            root: root.clone(),
            dir: tmp.path().join("snaps"),
        };
        let plan = s
            .plan(
                &SnapshotInfo { path: snap.clone() },
                &[root.join("inbox"), home.clone()],
            )
            .unwrap();
        assert_eq!(
            plan.recreate,
            vec![PathBuf::from("inbox/a.pdf"), PathBuf::from("inbox/b.jpg")]
        );
        assert!(plan.notes.is_empty(), "{:?}", plan.notes);
        let elsewhere = tmp.path().join("elsewhere");
        let plan = s.plan(&SnapshotInfo { path: snap }, &[elsewhere]).unwrap();
        assert!(plan.recreate.is_empty());
        assert_eq!(plan.notes.len(), 1);
    }

    #[test]
    fn protected_paths() {
        assert!(is_protected(Path::new(".claude/settings.json")));
        assert!(is_protected(Path::new(".local/state/slate/audit.jsonl")));
        assert!(!is_protected(Path::new("src/main.rs")));
        assert!(!is_protected(Path::new(".claude2/x")));
    }

    #[test]
    fn the_oldest_snapshots_go_whatever_their_directories_say() {
        // Task ids: the time in hex, the daemon's process, a counter.
        let names = ["19ac0000003-4d2-2", "19ac0000001-4d2-0", "19ac0000002-91-0"];
        let all = || names.iter().map(|n| n.to_string());
        assert_eq!(beyond(all(), 3), Vec::<String>::new());
        assert_eq!(beyond(all(), 2), ["19ac0000001-4d2-0"]);
        assert_eq!(
            beyond(all(), 0),
            ["19ac0000001-4d2-0", "19ac0000002-91-0", "19ac0000003-4d2-2"]
        );
        // A longer number is a later time, not an earlier word.
        let later = ["100000000000-1-0", "fffffffffff-1-0"].map(String::from);
        assert_eq!(beyond(later.into_iter(), 1), ["fffffffffff-1-0"]);
    }

    #[test]
    fn what_slated_did_not_name_is_not_its_to_remove() {
        let names = [
            "mine",
            "19ac0000001-4d2-0",
            "backup-2026-11-01",
            "19ac0000002-4d2-1",
            ".keep",
        ];
        let left = beyond(names.iter().map(|n| n.to_string()), 1);
        assert_eq!(left, ["19ac0000001-4d2-0"]);
        assert_eq!(taken_at("19ac0000001-4d2-0"), Some(0x19ac0000001));
        for other in [
            "",
            "t1",
            "19ac0000001",
            "19ac0000001-4d2",
            "a-b-c-d",
            "zz-1-0",
        ] {
            assert_eq!(taken_at(other), None, "{other}");
        }
    }

    const MOUNTS: &str = "\
29 1 0:27 /@ / rw,noatime shared:1 - btrfs /dev/mapper/slateos rw,compress=zstd:3,ssd,space_cache=v2,subvolid=256,subvol=/@
31 29 0:27 /@home /home rw,noatime shared:5 - btrfs /dev/mapper/slateos rw,compress=zstd:3,ssd,user_subvol_rm_allowed,subvolid=258,subvol=/@home
33 29 0:27 /@nix /nix rw,noatime shared:7 - btrfs /dev/mapper/slateos rw,compress=zstd:3,ssd,space_cache=v2,subvolid=257,subvol=/@nix
40 29 0:35 / /home/a\\040b rw,relatime shared:9 - btrfs /dev/loop0 rw,ssd,subvolid=5,subvol=/
41 29 0:36 / /tmp rw,nosuid,nodev - tmpfs tmpfs rw,size=1024k
42 29 0:37 / /mnt/x rw - btrfs /dev/loop1 rw,ssd,subvolid=5,subvol=/
43 29 0:38 / /mnt/x rw - btrfs /dev/loop2 rw,ssd,user_subvol_rm_allowed,subvolid=5,subvol=/
";

    #[test]
    fn the_mount_that_holds_a_path_says_whether_snapshots_can_go() {
        let ask = |p: &str| mounted_with(MOUNTS, Path::new(p), REMOVAL_OPTION);
        let at = |p: &str, has: bool| Some((PathBuf::from(p), has));
        assert_eq!(
            ask("/home/alice/.local/state/slate/snapshots"),
            at("/home", true)
        );
        assert_eq!(ask("/var/lib/x"), at("/", false));
        // "/home/a b" is a mount of its own; "/homework" is not under "/home".
        assert_eq!(ask("/home/a b/snapshots"), at("/home/a b", false));
        assert_eq!(ask("/homework"), at("/", false));
        assert_eq!(ask("/tmp/x"), at("/tmp", false));
        // Two mounts on one point: the later one is what is there.
        assert_eq!(ask("/mnt/x/s"), at("/mnt/x", true));
        assert_eq!(mounted_with("", Path::new("/home"), REMOVAL_OPTION), None);
        assert_eq!(unescape("a\\040b\\134c\\"), "a b\\c\\");
    }

    /// The real thing, where there is one: `SLATE_TEST_BTRFS` names a btrfs subvolume
    /// this user owns. What is expected depends on how its filesystem is mounted.
    #[test]
    fn old_snapshots_are_removed_on_btrfs() {
        let Some(root) = std::env::var_os("SLATE_TEST_BTRFS").map(PathBuf::from) else {
            eprintln!("skipped: SLATE_TEST_BTRFS is not set");
            return;
        };
        let dir = root.join(format!("snapshots-{}", std::process::id()));
        let snap = Snapshotter::detect(root.clone(), dir.clone()).unwrap();
        // The source's own time says "long ago" for every snapshot taken of it.
        let names: Vec<String> = (0..5).map(|n| format!("19ac000000{n}-1-{n}")).collect();
        for name in names.iter().rev() {
            snap.create(name).unwrap();
        }
        let listed = || {
            let mut l: Vec<String> = fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            l.sort();
            l
        };
        let pruned = snap.prune(2);
        match snap.removal_blocked() {
            None => {
                assert_eq!(pruned.failed, None);
                assert_eq!(pruned.removed.len(), 3);
                assert_eq!(listed(), names[3..]);
                assert_eq!(snap.prune(2).removed.len(), 0);
                assert_eq!(snap.prune(0).removed.len(), 2);
                fs::remove_dir(&dir).unwrap();
            }
            Some(why) => {
                assert_eq!(pruned.failed.as_deref(), Some(why.as_str()));
                assert!(pruned.removed.is_empty());
                assert_eq!(listed(), names);
                // Left as they were: nothing may write into a snapshot.
                assert!(fs::write(dir.join(&names[0]).join("x"), "x").is_err());
                eprintln!("not removed, as expected here: {why}");
            }
        }
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
