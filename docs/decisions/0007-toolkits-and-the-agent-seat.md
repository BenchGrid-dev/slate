# 0007: Toolkits that only bind the first seat

- Status: accepted
- Date: 2026-09-27

## Context

The agent seat (ADR 0002) works by giving the agent a second `wl_seat`. Whether an application reacts to it depends on the toolkit binding every seat global, not just the first.

Measured on sway 1.12 with `WAYLAND_DEBUG=1`:

- foot (custom Wayland client): binds every seat, agent input works.
- GTK 4.22 (gnome-calculator): sees the transient seat global appear and disappear, never binds it. Only the first `wl_seat` is used. Clicks and keys from the agent seat have no effect.

GTK4 is most of GNOME. Qt and Chromium/Electron are untested and go on the compatibility list.

## Decision

Two paths, both explicit to the agent and the human:

1. **Agent seat first.** Default for every input tool. Costs the human nothing.
2. **User-seat fallback.** Input tools take `seat: "user"`, which injects through the human's own seat (virtual pointer/keyboard bound to seat0). slated classifies this as Confirm: the human is told the agent wants to borrow their mouse and keyboard for the action. Tool descriptions tell the agent to retry with `seat: "user"` when agent-seat input has no visible effect.

Longer term, the compositor closes the gap without client cooperation: apps the agent launches connect through a `wp_security_context_v1` socket, and a patched sway filters the globals those clients see so that the agent seat is their *only* seat. Human takeover then means attaching the human's physical devices to the agent seat (`swaymsg seat <name> attach`), which no client can tell from normal input. Pre-existing windows of first-seat-only toolkits still need the fallback.

## Consequences

- The clean "agent never touches your input" story holds for well-behaved clients and for anything the agent launches on a patched compositor, not for pre-existing GTK4 windows on stock sway.
- Compositor patches are now a planned part of Slate OS, not an optional nicety.
- A per-toolkit compatibility list lives in the slate-desktop README.
