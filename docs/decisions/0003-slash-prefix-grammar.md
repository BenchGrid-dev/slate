# 0003: slash prefix grammar

- Status: accepted
- Date: 2026-09-26

## Context

slash is the primary interface of the OS, and most of its users already use Claude Code or Codex, which share a prefix convention.

## Decision

- Bare text goes to the agent backend.
- `/` is a control command: handled by slash if recognised, otherwise passed to the backend.
- `!` runs the rest of the line in a persistent zsh/bash session.
- `@` inside a message references a file.

No heuristic routing of bare text to the shell. slash is not intended to replace zsh for people who want zsh; it is the interface for people who do not want a shell at all. `!` is the escape hatch, not the daily path.

## Consequences

- Zero learning curve for Claude Code and Codex users.
- Absolute paths starting with `/` are ambiguous; use `!/path/to/bin`.
- slash must exec the POSIX shell when invoked non-interactively so `$SHELL -c` keeps working.
