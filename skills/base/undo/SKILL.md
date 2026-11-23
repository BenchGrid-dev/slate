---
name: slate-undo
description: How Slate snapshots, undo, and the audit log work. Use when the user asks to undo, revert, or wants to know what the agent changed.
---

# Undo, snapshots and audit in Slate

Slate takes a read-only btrfs snapshot of the user's home before the first change a task makes. The user can say "undo" at any time.

- `slate undo --preview` shows what undo would restore, delete or recreate (tier: observe).
- `slate undo` applies it (tier: reversible; it is itself undoable only by re-doing the work).
- `slate audit 50` shows what recent tasks did; `slate tasks` lists tasks and whether they have a snapshot.

Slate keeps the snapshots of the last 30 tasks and removes older ones itself; an older task can no longer be undone. They live in `~/.local/state/slate/snapshots` and are not yours to delete. When `slate status` says "old snapshots are not removed", they pile up and take disk space: tell the user what it says (on SlateOS, `systemctl status slate-snapshot-removal` shows why the mount option is missing).

If the user asks "what did you change?", answer from `slate audit`, not from memory. If snapshots are unavailable (`slate status` says so), say that changes cannot be undone automatically before making any.
