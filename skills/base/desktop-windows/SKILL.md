---
name: slate-desktop-windows
description: Manage desktop windows on Slate (open, arrange side by side, move, resize, maximise, focus, close) and drive GUI apps through the agent's own seat. Use for any request about windows, apps on screen, or the desktop layout.
---

# Desktop windows on Slate

You have MCP tools under the `desktop` server. Prefer them over keyboard shortcuts; the window manager's shortcuts differ per machine and keys you guess land in whatever window is focused.

| Want | Use |
|---|---|
| see what is open | `desktop_windows` (ids, app_id, title, position, size) |
| open an app | `desktop_launch` (`firefox`, `foot`, `thunar`, `gnome-text-editor`, `gnome-calculator`) |
| two or more windows side by side / stacked / grid | `desktop_arrange` with `layout` and an ordered `windows` list |
| maximise one window | `desktop_arrange` layout `maximize`, or `desktop_window_set` fullscreen |
| move or resize | `desktop_window_set` with x, y, width, height |
| bring to front | `desktop_focus` |
| close | `desktop_close` (never guess a shortcut) |
| look at a window | `desktop_screenshot` with the window; coordinates in the image are the ones `desktop_click` takes |
| type into an app | `desktop_type` / `desktop_key` **with `window` set**: the tool focuses that window for the seat in use, verifies, and reports where the text went. Never type without naming the window. |

After `desktop_click`, `desktop_type` and `desktop_key` you get a screenshot of the window taken right after the action. Look at it: only report "clicked X" or "typed Y" if the screenshot shows the effect (a dialog closed, text in the field, a page changed). If nothing changed, say so and try something else (a different spot, `seat: "user"`, or a different approach) rather than claiming success.

Tier: reversible. Closing a window may lose unsaved work: say so before closing an editor or browser with unsaved changes.

The Slate desktop profile's own shortcuts, for when the user asks: Mod+Return terminal, Mod+Space launcher, Mod+w browser, Mod+e files, Mod+t editor, Mod+q close, Mod+1..5 workspaces, Mod+f fullscreen. Mod is the Super key.
