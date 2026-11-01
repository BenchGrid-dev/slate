# Policy file

`~/.config/slate/policy.toml` adjusts how slated classifies tool calls. Everything is optional. Built-in rules still apply where the file is silent; unknown tools stay Confirm.

```toml
[tools]
# exact tool name -> "observe" | "reversible" | "confirm"; wins over every other rule
"mcp__github__create_issue" = "confirm"
"mcp__myserver__lookup" = "observe"

[shell]
read_only = ["mytool", "kubectl get"]      # extra commands treated as read-only
confirm_patterns = ["deploy ", "terraform"] # extra substrings that force Confirm
trusted_patterns = ["git push origin feature/"] # substrings that bypass Confirm (careful)

[paths]
sensitive = ["/srv/vault", "/etc/wireguard"] # writes here need Confirm
```

The file is read once when slated starts. Restart slated (or `pkill slated`; slash restarts it) after editing.

The tiers: **observe** runs silently, **reversible** runs after a snapshot and can be undone, **confirm** stops and asks the human.

A shell command that deletes, moves away or truncates files (`rm`, `rmdir`, `unlink`, `shred`, `mv`, `truncate`, `find … -delete`) is reversible only when undo could bring them back: every path it names lies inside the snapshot root and outside what undo leaves alone (Slate's own state, agent and package caches). Otherwise, and on a system without snapshots, it is confirm, and the request says which path undo could not restore. A path slated cannot work out without running the shell (`$DIR/x`, `$(…)`) counts as one undo could not restore.
