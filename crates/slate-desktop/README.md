# slate-desktop

Background computer use for Linux. Gives an agent its own Wayland seat (pointer, keyboard, focus, clipboard), per-window screenshots, and input, exposed over MCP so Claude Code and Codex can drive GUI apps without touching the human's mouse.

## Status

Works, verified on sway 1.12 (headless and on a real display, HiDPI included); the SlateOS desktop runs it as a supervised user service. Needs a compositor that implements:

- `ext_transient_seat_manager_v1`
- `zwlr_virtual_pointer_manager_v1` (v2) and `zwp_virtual_keyboard_manager_v1`
- `ext_image_copy_capture_manager_v1` with `ext_foreign_toplevel_image_capture_source_manager_v1` for per-window capture
- `ext_foreign_toplevel_list_v1`

That is wlroots-based compositors today. GNOME and KDE do not expose these to clients. Window geometry (for window-relative clicks) comes from sway IPC; on other compositors screenshots and absolute coordinates still work.

## The daemon

`slate-desktop daemon` owns the agent seat for the whole session and must start before any application: toolkits only accept input from seats that existed when they started. The Slate sway profile runs it first (`exec slate-desktop daemon`). `slate-desktop serve` (MCP) and the CLI use the daemon when it is running and fall back to a private seat otherwise.

## Borrowing the user's seat

`seat: "user"` actions put sway into a `controlling` binding mode (the profile binds Esc there to `slate-desktop takeover-cancel`), the panel shows a blinking "controlling", and after Esc further user-seat input is refused for a minute so the agent has to ask again. `desktop_status` reports the state.

## MCP tools

`desktop_windows`, `desktop_screenshot [window]`, `desktop_click`, `desktop_move`, `desktop_scroll`, `desktop_type`, `desktop_key`, `desktop_close`, `desktop_focus`, `desktop_window_set`, `desktop_arrange`, `desktop_launch`, `desktop_seats`, `desktop_status`, `desktop_takeover_cancel`. Window-relative coordinates match window screenshots, so an agent can look, then click what it saw. `desktop_type` and `desktop_key` take a `window`, focus it for the seat in use, verify the focus through the compositor and report which window received the input. Every action returns a screenshot of the result.

slash adds this server to the agent backend automatically when `WAYLAND_DISPLAY` is set and the binary sits next to `slash`.

## Toolkit compatibility

Whether an app reacts to the agent seat depends on its toolkit binding every `wl_seat`, not just the first. Verified on sway 1.12:

| Toolkit / app | Agent seat | Notes |
|---|---|---|
| foot | works | binds all seats |
| Firefox, GTK3 (Thunar) | works | when the seat predates the app (the daemon guarantees that) |
| GTK 4 / libadwaita (gnome-text-editor, gnome-calculator) | ignored | only the first seat is bound; use `seat: "user"` |
| Qt, Chromium/Electron, Flatpak | untested | contributions welcome |

`seat: "user"` injects through the human's own seat and needs approval (slated tier Confirm). Pointer and keyboard both work this way on a real display. See `docs/decisions/0007-toolkits-and-the-agent-seat.md` for the compositor-side fix that removes the need for it.

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
- Takeover beyond Esc (freeze the seat, hand a window back)
- Multiple outputs
