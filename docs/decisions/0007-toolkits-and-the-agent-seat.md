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

Measured: with `seat: "user"`, keyboard events reach GTK4; on a headless sway whose seat0 has no physical pointer, `wl_pointer.button` events from the virtual pointer were not delivered to any client (enter/leave were). Whether a real desktop with a physical mouse behaves differently is an open item.

Measured later the same day (Firefox 156, GTK3): an app accepts pointer and keyboard input from the agent seat only if that seat already existed when the app started. Seats created later are bound (the protocol log shows the bind) but their input is ignored. This is why per-turn transient seats failed on the user's Firefox while the e2e suite, whose Firefox is a child of the seat owner, passed.

**Amendment:** the agent seat is owned by one long-lived `slate-desktop daemon` per session, started by the compositor before any application. The MCP server and the CLI proxy to it. Apps started before the daemon (or before login) still need the user-seat fallback.

## Consequences

- The clean "agent never touches your input" story holds for well-behaved clients and for anything the agent launches on a patched compositor, not for pre-existing GTK4 windows on stock sway.
- Compositor patches are now a planned part of SlateOS, not an optional nicety.
- A per-toolkit compatibility list lives in the slate-desktop README.
