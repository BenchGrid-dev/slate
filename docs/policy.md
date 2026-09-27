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
