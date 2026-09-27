# 0006: Privilege-free snapshots

- Status: accepted
- Date: 2026-09-27

## Context

slated snapshots the user's files before an agent changes them, so "undo" always works. On btrfs, `subvolume snapshot` of a subvolume you do not own needs CAP_SYS_ADMIN, and so do `find-new` and `subvolume show`. Giving the daemon root, or a passwordless sudo rule for `btrfs`, would make the component that agents talk to a privilege boundary. We do not want that.

Measured on the dev VM (NixOS 26.05, kernel 6.18): an unprivileged user can create a subvolume, take read-only snapshots of a subvolume they own, and delete those snapshots. They cannot run `find-new` or `subvolume show`.

## Decision

- The snapshot root must be a btrfs subvolume owned by the user. Slate OS creates every user's home as its own subvolume at install / user-creation time. On other systems, `SLATE_SNAPSHOT_ROOT` can point at a user-owned subvolume and undo covers only what is under it.
- slated runs as the user with no extra privileges. It never calls sudo.
- Change detection diffs the snapshot against the live tree inside the directories the task touched (size, mtime, symlink target), instead of using `find-new`.
- Slate's own state and agent caches are never rolled back.

## Consequences

- Undo scope is "the directories the task worked in", not "the whole home". This is what a user expects from "undo that", and it keeps the diff bounded.
- Files changed outside the snapshot root are not undoable; the policy classifies those writes as Confirm.
- Detecting subvolumes uses inode 256 plus the btrfs statfs magic, which needs no privileges.
- On non-btrfs systems slated still runs; approvals and audit work, undo reports that snapshots are unavailable.
