# Decisions

Architecture decision records. Short, dated, and never edited after the fact; a reversed decision gets a new record that supersedes the old one.

| # | Decision | Status |
|---|---|---|
| 0001 | Bring your own agent: no model API calls, official binaries only | accepted, amended by 0008 |
| 0002 | Agents get their own Wayland seat | accepted |
| 0003 | slash prefix grammar: bare, `/`, `!`, `@` | accepted |
| 0004 | Base distribution | open |
| 0005 | Rust for all first-party components | accepted |
| 0006 | Privilege-free snapshots: each user's home is a user-owned btrfs subvolume | accepted |
| 0007 | Toolkits that only bind the first seat: user-seat fallback now, compositor global filtering later | accepted |
| 0008 | API connections: the person's own key and endpoint, through a gateway of Slate's own | accepted |
| 0009 | Software update: a release replaces the source, the system is built again and started once on trial | accepted |
| 0010 | The agent's programs are downloaded by each computer, not shipped on the image | accepted |
| 0011 | Codex's tool calls reach Slate through the system's hooks; its sandbox is the person's choice | accepted |
| 0012 | The choices that lower a guard are the system's, behind the person's password | accepted |
