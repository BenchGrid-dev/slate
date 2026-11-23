# 0011: Codex's tool calls reach Slate through the system's hooks; its sandbox is the person's choice

- Status: accepted
- Date: 2026-11-22

## Context

Slate's approvals, snapshots and audit log hang on one thing: the agent's program tells slated about every call before it makes it. Claude Code does, through a PreToolUse hook and a permission tool that slash hands it for each run. Codex did not (0008 names it as the reason the gateway serves Claude Code): slash started `codex exec` under Codex's own sandbox, and slated recorded that a task began and ended. With 0.3 a ChatGPT subscription is a choice in the installer (0010), so the agent a person ends up with may be the one that never asks and cannot be undone.

Codex has hooks of the same shape now. Tried against Codex 0.146, the version SlateOS installs, signed in with a ChatGPT account:

- `codex exec` calls a PreToolUse hook before every shell command (as `Bash`), every file edit (`apply_patch`, the patch as its `command`) and every MCP tool (`mcp__server__tool`), and waits for it, minutes if need be. The hook has the environment of the process that started Codex. A `deny` stops the call and the reason reaches the model.
- A hook's `ask` is not acted on, and in `exec` nothing of Codex's own asks a person.
- Codex runs a hook only if the person has reviewed it in Codex's own interface, or if it is the system's: `/etc/codex/hooks.json`. Hooks given on the command line are skipped.
- One flag lifts the review for a run, `--dangerously-bypass-hook-trust`. With it Codex also ran the hooks of a `.codex/hooks.json` lying in the folder it was started in, a folder never marked as trusted.

## Decision

**SlateOS installs Slate's hooks as the system's.** The module writes `/etc/codex/hooks.json`: `slate hook codex-pre-tool-use` before a call, `slate hook codex-post-tool-use` after it. slash passes nothing; Codex finds them. Elsewhere `slate hook codex-config` prints the file for whoever administers the machine.

**slash does not use the flag.** It would make every folder a person asks a question in a place commands can come from, outside the sandbox and before any approval. A downloaded archive with a `.codex` folder in it would be enough.

**The hook asks the person itself.** slated classifies the call as it does Claude Code's. What needs no word passes in silence, after the snapshot where one is due. What needs the person's word is put to them by the hook, through slated, in slash or on the desktop; the hook waits for the answer and refuses the call when it is no.

**Without the daemon a task gets nothing through.** Claude Code falls back on its own permissions when slated is gone. Codex has none in `exec`, so its hook refuses.

**The hook does nothing when Codex is not working for slash.** It is the system's, so it also starts when someone runs Codex themselves. Without a Slate task it exits at once.

**Codex starts in its sandbox, and the person can let it out.** `[codex] sandbox` in slash.toml stays `workspace-write` by default: commands write under the folder Codex works in, reach no network and cannot become root, whatever Slate would allow. Settings, AI has the switch (it writes `danger-full-access`), behind a question that says what is given up. Claude Code runs without a sandbox on SlateOS; a person who chose Codex can have the same.

**The sandbox is lifted only where the hooks work.** Outside it the hook is the only guard, so slash keeps Codex in `workspace-write`, whatever the setting, when the system's hooks file is missing or slated does not follow the task (a turn without a Slate task looks to the hook like Codex used by itself, and passes). slash's first line says so.

**Slate's own settings are asked about.** The files under `~/.config/slate` decide whether approvals are asked, by which rules, and now whether Codex is sandboxed; they lie in the person's home, where either agent could write them as an ordinary, reversible edit. A change to them, by any tool, now needs the person's word. Reading stays free.

## Consequences

- With Codex, approvals, the audit log and undo now work as with Claude Code for what Codex can do: files under the folder it works in, and the desktop, whose tools run outside the sandbox and were allowed wholesale before.
- Until the person lets it out, Codex on SlateOS does less than Claude Code. It cannot install software, change system settings (sudo does not work inside the sandbox) or fetch anything from a command. Let out, it can; Slate's hooks are then the only guard, and Codex's documentation calls hooks a guardrail, not a complete enforcement boundary: tools it runs on OpenAI's side, web search among them, do not pass them. Claude Code's permissions are a gate every call goes through. The question in Settings says this.
- The switch is a click, like "Bypass approvals by default" beside it. An agent that drives the desktop could make that click on its own screen; the choices about what the agent may do while the person is away are behind the password for that reason, and these two are not.
- With Claude Code too, changing `slash.toml` or `policy.toml` for the person ("use the other model") now asks first.
- The file is the system's, so a system without it (Slate from source on another distribution) runs Codex as before, and slash's first line says that nothing is asked or undone there.
- A tool of Codex's that the policy does not know is asked about (the policy's rule for unknown tools), which may be more often than it deserves until the policy learns its name.
- `codex login status` goes on saying "logged in" after the sign-in has expired; only a turn finds out. The connection form cannot tell either.

## Alternatives considered

- **Handing the hooks over for each run with the bypass flag.** Works everywhere without a file in `/etc`, and runs strangers' hooks (above).
- **Recording Slate's hook as reviewed in the person's Codex configuration.** Codex keeps that review against the hook's content in a format of its own; writing it for the person is forging their consent.
- **A snapshot at the start of every Codex task, and no hooks.** Undo without approvals. It would snapshot for tasks that change nothing, and still let Codex use the person's mouse and keyboard unasked.
- **No sandbox for Codex from the start.** The same abilities as Claude Code at once. The sandbox costs a person nothing until they miss something, and what replaces it is weaker than what guards Claude Code (above), so it is theirs to give up.
