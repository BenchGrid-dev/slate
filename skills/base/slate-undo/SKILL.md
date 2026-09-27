# Undo, snapshots and audit in Slate

Slate takes a read-only btrfs snapshot of the user's home before the first change a task makes. The user can say "undo" at any time.

- `slate undo --preview` shows what undo would restore, delete or recreate (tier: observe).
- `slate undo` applies it (tier: reversible; it is itself undoable only by re-doing the work).
- `slate audit 50` shows what recent tasks did; `slate tasks` lists tasks and whether they have a snapshot.

If the user asks "what did you change?", answer from `slate audit`, not from memory. If snapshots are unavailable (`slate status` says so), say that changes cannot be undone automatically before making any.
