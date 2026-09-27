---
name: slate-systemd-services
description: Inspect, start, stop, restart, and read logs of systemd services. Use for anything about services, daemons, units, or journal logs. Linux with systemd only.
---

# systemd services

## Look (tier: observe)

```
systemctl status <unit>
systemctl --user status <unit>        # per-user services, e.g. slated
systemctl list-units --type=service --state=failed
journalctl -u <unit> -n 100 --no-pager
journalctl --user -u <unit> -n 100 --no-pager
```

## Change (tier: confirm)

```
systemctl restart <unit>
systemctl stop <unit>
systemctl enable --now <unit>
```

Stopping or restarting a service can interrupt what the user is doing (network, audio, display). Say which unit and why before asking. Per-user units (`--user`) affect only this user.

## Verify

`systemctl is-active <unit>` prints `active`.
