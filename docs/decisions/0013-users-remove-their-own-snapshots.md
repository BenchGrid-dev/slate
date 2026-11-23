# 0013: Old snapshots are removed by the user: the filesystem is mounted to allow it

- Status: accepted
- Date: 2026-11-22
- Amends: 0006

## Context

0006 has slated take snapshots and remove them as the user, without privileges, and says that both were measured. The second was not so. On the dev VM (kernel 6.18) and on every installed system, `btrfs subvolume delete` by the user answers "Operation not permitted". slated kept "the last 30 snapshots" in name only: the removal failed, nothing reported the failure, and every task since the first left its snapshot behind. The dev VM held 76.

A snapshot costs nothing when it is taken and then holds on to everything that is changed or deleted after it. The disk fills slowly, for no reason a person could find.

Measured again on kernel 6.18, on a scratch filesystem:

- Without the mount option `user_subvol_rm_allowed`, only root deletes a subvolume.
- With it, a user deletes a subvolume in a directory they may write to, when the subvolume is writable. A read-only snapshot answers "Read-only file system". Its owner may make it writable (`btrfs property set ro false`) without privileges.
- A subvolume in a directory that is root's, like a home under `/home`, is not its owner's to delete, with or without the option.
- A remount adds the option. It is the whole filesystem's: every mount of the filesystem shows it, and later remounts keep it.
- A snapshot's directory carries the times of what it was taken of (modification, change and birth): they say when the home's top folder last changed, not when the snapshot was taken. slated ordered snapshots by them, so a removal that worked would have removed in the wrong order.

## Decision

**slated still removes snapshots itself, as the user.** Before it deletes one it makes it writable; when the deletion fails it makes it read-only again.

**SlateOS mounts the filesystem so that it can.** A service at every start (`slate-snapshot-removal`) remounts the filesystem that holds each person's home, and `snapshotRoot`, with `user_subvol_rm_allowed`. `services.slate.snapshotRemoval = false` leaves it out.

**A snapshot's age is read from its name.** slated names a snapshot after its task, and a task's id begins with the time it started. What lies in the snapshot directory under another name is not slated's and stays.

**Removal waits for nobody and hides nothing.** It runs on a thread of its own when the daemon starts and after each snapshot, so no tool call waits for it. Where it cannot work, slated says so once in its log, and `slate status` says it for as long as it is true.

## Consequences

- With the option, any user may delete a subvolume they own in a directory they may write to. On a desktop these are the person's own snapshots and the subvolumes they made. Their home is not among them, and nothing of root's is.
- The agent runs as the person, so it could delete a snapshot too. It could already make one writable and change it; a `btrfs` command that is not a listing is asked about either way.
- Systems installed before this get the option with the update, because the service is the module's: `hardware-configuration.nix` stays as it was written. The first start afterwards removes what had piled up.
- Elsewhere (slash from source on another system) the person adds the option to the mount. `slate status` names the mount.

## Alternatives considered

- **A helper with root's rights that deletes snapshots** (a sudo rule, or a system service). 0006's reason stands: the daemon agents talk to would become a privilege boundary, and a root process that deletes what a user's directory names has symbolic links and races to get right.
- **Emptying the snapshot file by file, then `rmdir`.** An empty subvolume is its owner's to remove without any option, so this works everywhere. But it rewrites all of the snapshot's metadata only to throw it away: seconds to minutes of disk writes with every task.
- **The option in `fileSystems`.** The declarative way. But the module cannot ask which mount holds the homes without reading the option it would be defining, and systems already installed have a hardware configuration of their own. A remount needs to know neither.
- **Writable snapshots from the start.** One command less; and nothing would keep a stray write out of what undo reads.
