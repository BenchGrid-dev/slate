# 0012: The choices that lower a guard are the system's, behind the person's password

- Status: accepted
- Date: 2026-11-22

## Context

Some settings decide how much stands between the agent and the machine, or between the machine and whoever walks up to it: whether the agent's risky actions are asked about at all ("Bypass approvals by default"), whether Codex keeps its sandbox (0011), when the screen locks. Up to 0.2 they were values in the person's own files (`slash.toml`, `idle.json`) behind plain switches in Settings.

The agent runs as the person. It could write those files as ordinary, reversible edits, and it drives the desktop, so it could also open Settings on its own screen and click. A request planted in a web page ("turn off approvals first") would have had nothing in its way. Two choices of the same kind were already kept differently since 0.2: what the agent may do while the screen is locked, and whether the screen locks while it works, are root's (`slateos-settings`), and Settings asks for the password to change them.

## Decision

**A choice that lowers a guard is kept where only root writes.** `/etc/slateos/settings.json`, through `slateos-settings set`: `bypass-approvals`, `codex-sandbox`, `lock-after`, beside the ones that were there. What reads them (slash, the session's idle timer) reads them from there.

**Settings asks for the person's password for every change of one**, in either direction, with what the change means in the dialog (`sudo -k -A`: never a password given a moment ago for something else). The agent does not know the password and cannot type it. This is how System Settings on macOS treats its sensitive panes.

**On SlateOS, slash.toml cannot make these choices.** `auto_approve = true` and `sandbox = "danger-full-access"` there are not used, and slash says so at start. Where the system holds no such choices (slash from source on another system), slash.toml decides, as before.

**A change to Slate's own files in `~/.config/slate` is asked about** (0011), for what remains there: the rules in `policy.toml`, the agent's connection.

## Consequences

- An update from 0.2 puts these three back to their defaults: approvals asked, Codex sandboxed, the lock after ten minutes. Whoever had chosen otherwise chooses again, once, with the password.
- `/auto on`, typed in a session, still bypasses approvals for that session. It is the person's word in their own terminal, and ends with the session.
- The password guards the settings. It does not make the agent another user: a program running as the person can still stop the session's idle timer, or speak to slated directly. Separating the agent's identity from the person's is the larger piece of work this leaves open.
- How the agent reaches a model (the API address and key, the sign-ins) stays the person's own file and form. Its file is asked about like the others; the form is not behind the password.
- When the screens turn off, the model names, the verbose log and the like stay plain switches: nothing is lost when they change.

## Alternatives considered

- **A password dialog in Settings over the same files.** It would stop a click and nothing else; the files would be as writable as before. A lock that only one of two doors has.
- **polkit actions instead of sudo.** The same guarantee with a second mechanism to carry; sudo with Slate's password dialog is what the other root-owned choices already use.
- **Asking only when a guard is lowered.** Friendlier by one dialog. But the files are root's either way, and a rule with a direction in it is one more thing to get wrong.
