# slate-desktop

Background computer use for Linux. Gives an agent its own Wayland seat (pointer, keyboard, focus, clipboard), per-window screenshots, and input, exposed over MCP so Claude Code and Codex can drive GUI apps without touching the human's mouse.

## Status

v0, verified on sway 1.12 (headless and normal). Needs a compositor that implements:

- `ext_transient_seat_manager_v1`
- `zwlr_virtual_pointer_manager_v1` (v2) and `zwp_virtual_keyboard_manager_v1`
- `ext_image_copy_capture_manager_v1` with `ext_foreign_toplevel_image_capture_source_manager_v1` for per-window capture
- `ext_foreign_toplevel_list_v1`

That is wlroots-based compositors today. GNOME and KDE do not expose these to clients. Window geometry (for window-relative clicks) comes from sway IPC; on other compositors screenshots and absolute coordinates still work.

## MCP tools

`desktop_windows`, `desktop_screenshot [window]`, `desktop_click`, `desktop_move`, `desktop_scroll`, `desktop_type`, `desktop_key`, `desktop_launch`. Window-relative coordinates match window screenshots, so an agent can look, then click what it saw. Screenshots come back as PNG image content.

slash adds this server to the agent backend automatically when `WAYLAND_DISPLAY` is set and the binary sits next to `slash`.

## Toolkit compatibility

Whether an app reacts to the agent seat depends on its toolkit binding every `wl_seat`, not just the first. Verified on sway 1.12:

| Toolkit / app | Agent seat | Notes |
|---|---|---|
| foot | works | binds all seats |
| GTK 4.22 (gnome-calculator) | ignored | only the first seat is bound; use `seat: "user"` |
| Qt, Chromium/Electron, GTK3 | untested | contributions welcome |

`seat: "user"` injects through the human's own seat and needs approval (slated tier Confirm). See `docs/decisions/0007-toolkits-and-the-agent-seat.md` for the compositor-side fix that removes the need for it.

## CLI (for testing)

```
slate-desktop probe
slate-desktop windows
slate-desktop shot [IDENT] out.png
slate-desktop click X Y [left|right|middle]
slate-desktop type "echo hi"
slate-desktop key Return
slate-desktop launch foot
slate-desktop --seat user click 700 600     # borrow the human's seat (GTK4 apps)
```

Each CLI call creates a fresh transient seat; `serve` keeps one for the life of the MCP session.

## How typing works

Arbitrary Unicode is typed by generating an xkb keymap where every distinct character in the text has its own keycode (the wtype approach), uploading it to the virtual keyboard, then pressing those codes. Named keys and modifiers live in the same keymap so `ctrl+l` and `Return` work. One quirk: the first key event after a keymap upload is dropped between the compositor and the client, so a reserved no-op key is pressed after every upload.

## Not yet

- A distinct "ghost" cursor for the agent seat (needs compositor support or a patch)
- Headless output for agent windows the user does not see
- Accessibility tree (AT-SPI2) so agents can act on named controls instead of pixels
- Human takeover controls (freeze the seat, hand a window back)
- Multiple outputs
