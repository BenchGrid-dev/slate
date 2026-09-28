# 0002: Agents get their own Wayland seat

- Status: accepted
- Date: 2026-09-26

## Context

"Humans and agents co-exist" has to mean something concrete on a desktop. If an agent injects input into the user's seat, the user cannot work while the agent works: the pointer moves, focus jumps, the clipboard is clobbered. Codex on macOS solves this with private APIs. Wayland has standard protocols for it.

## Decision

Every agent task runs against its own seat, created with ext-transient-seat-v1, with virtual pointer and keyboard bound to that seat. Per-window capture uses ext-image-copy-capture-v1. The agent's cursor is rendered distinctly. The user can freeze, take over or reveal the agent's work at any time.

## Consequences

- Slate targets Wayland only. X11 is not a supported target.
- Slate targets compositors that implement these protocols: wlroots-based today. GNOME and KDE are not supported until they do.
- SlateOS ships and, where needed, patches its own compositor.
- The protocols are standard, so slate-desktop should work on any conforming compositor with reduced integration.
