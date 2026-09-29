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

## The background screen

On the SlateOS desktop sway has a second, headless output (`HEADLESS-1`, workspace `agent`). `desktop_launch` opens windows there unless `where: "here"`; `desktop_show` and `desktop_hide` move windows between the screens; `desktop_windows` shows each window's `location`. The agent seat has one virtual pointer per output and routes clicks by layout coordinates, so windows on either screen can be driven the same way. Without a headless output (plain sway), everything opens on the user's screen.

## The accessibility tree

`desktop_elements`, `desktop_read`, `desktop_element_click` and `desktop_element_set_text` work through AT-SPI2, the accessibility bus every Linux toolkit speaks. Elements come with a role, a name, a value, states, window-relative extents and their actions; text comes back as text. `desktop_element_click` runs the element's own action (`Action.DoAction`) and `desktop_element_set_text` uses `EditableText`, so neither needs a pointer or a keyboard and both work in toolkits that ignore extra seats (GTK4). Pointer and typing fallbacks kick in when an element has no action or is not editable over the bus.

Applications are matched to compositor windows by pid (sway reports the pid of each view; the bus reports the pid of each connection), frames by title. Trees are read from the application's `Cache` in one call, with a walk from the frame when the cache is shallow (GTK4 caches lazily). Verified: Thunar and pavucontrol (GTK3), the settings app (GTK4/libadwaita), Firefox (about 2000 nodes in under three seconds).

Two switches must be on, and the SlateOS desktop profile sets both: the bus must say accessibility is enabled (`org.a11y.Status IsEnabled`, mirrored from the `toolkit-accessibility` gsettings key; the daemon also sets it at connect), and `NO_AT_BRIDGE` must not be `1` (NixOS sets it unless `services.gnome.at-spi2-core` is enabled). Firefox and Chromium additionally read `GNOME_ACCESSIBILITY` / `ACCESSIBILITY_ENABLED`, Qt reads `QT_LINUX_ACCESSIBILITY_ALWAYS_ON`.

## MCP tools

`desktop_windows`, `desktop_elements`, `desktop_read`, `desktop_element_click`, `desktop_element_set_text`, `desktop_screenshot [window]`, `desktop_click`, `desktop_move`, `desktop_scroll`, `desktop_type`, `desktop_key`, `desktop_close`, `desktop_focus`, `desktop_window_set`, `desktop_arrange`, `desktop_launch` (with `where`), `desktop_show`, `desktop_hide`, `desktop_seats`, `desktop_status`, `desktop_takeover_cancel`. Window-relative coordinates match window screenshots, so an agent can look, then click what it saw. `desktop_type` and `desktop_key` take a `window`, focus it for the seat in use, verify the focus through the compositor and report which window received the input. Every action returns a screenshot of the result.

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
slate-desktop elements WINDOW [QUERY]
slate-desktop read WINDOW
slate-desktop element-click ID [ACTION]
slate-desktop element-set-text ID TEXT
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
- Takeover beyond Esc (freeze the seat, hand a window back)
- Multiple outputs
