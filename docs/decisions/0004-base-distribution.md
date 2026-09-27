# 0004: Base distribution

- Status: open
- Date: 2026-09-26

## Context

Slate OS needs a base. Two candidates:

- **NixOS.** Declarative system state. An agent changing the system produces a text diff that can be reviewed and rolled back by generation. Snapshots and undo of system configuration come for free. Smaller community, steeper learning curve, some apps harder to package.
- **Arch.** Largest desktop app surface, AUR, a validated precedent in Omarchy. Rollback needs btrfs snapshots layered on top. Imperative state is harder for an agent to reason about.

## Decision

Not yet made. The M1 prototype runs on whatever the contributor has, so this does not block work. An RFC is requested.

## Leaning

NixOS, because "the agent edits a text file and the OS rebuilds" is the same mental model as vibe coding, and generation rollback is exactly the undo story slated wants. The cost is packaging and onboarding friction; that cost is real and needs to be argued out.
