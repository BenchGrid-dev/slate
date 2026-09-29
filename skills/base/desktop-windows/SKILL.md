---
name: slate-desktop-windows
description: Manage desktop windows on SlateOS (open, arrange, move, resize, focus, close) and drive GUI apps through the accessibility tree first and the agent's own seat second. Use for any request about windows, apps on screen, or doing something inside a desktop application.
---

# Desktop windows and applications on SlateOS

You have MCP tools under the `desktop` server. Prefer them over keyboard shortcuts; the window manager's shortcuts differ per machine and keys you guess land in whatever window is focused.

## Where to work: the user's screen or your own

You have a second screen the user never sees (the "background"). Decide per task:

| The task | Where |
|---|---|
| Delivers a result: convert, download, fill in, look something up, prepare a document, run a long job | **background** (the default of `desktop_launch`). The user keeps their screen; you hand over the result. |
| Helps with what the user is doing now: "this window", "the page I have open", "click that", fix the spreadsheet they are looking at | **here**: act on their window in place, elements first; `desktop_launch` with `where: "here"` if you must open something for them to see. |
| The user says "show me" / "let me see" | `desktop_show` (all background windows, or one). |
| The user says "get that out of my way" / "do it in the background" | `desktop_hide`. |

Open applications with `desktop_launch`, never from the shell (`firefox &`, `xdg-open`, `libreoffice file.odt` in a terminal command). A program started from the shell appears on the user's screen and takes their focus for a moment before the desktop moves it to the background; `desktop_launch` opens it in the right place from the start and gives you the window id.

When unsure, use the background: a task done there and shown at the end costs the user nothing; a window popping up over their work interrupts them. Windows already on the user's screen stay there unless they ask; do not raise or focus them for your own convenience (`desktop_focus` also takes the user's keyboard focus). Say where you worked when you report: "prepared in the background, `desktop_show` to see it".

## Windows

| Want | Use |
|---|---|
| see what is open | `desktop_windows` (ids, app_id, title, position, size) |
| open an app | `desktop_launch` (`firefox`, `foot`, `thunar`, `libreoffice`, `thunderbird`, `mousepad`, `zathura`, `imv`, `mpv`); `where` picks background (default) or here |
| show / hide background windows | `desktop_show` / `desktop_hide` |
| two or more windows side by side / stacked / grid | `desktop_arrange` with `layout` and an ordered `windows` list |
| maximise one window | `desktop_arrange` layout `maximize`, or `desktop_window_set` fullscreen |
| move or resize | `desktop_window_set` with x, y, width, height |
| bring to front on the user's screen | `desktop_focus` (takes their keyboard focus: only when they asked) |
| close | `desktop_close` (never guess a shortcut) |

## Inside an application: elements first, pixels second

1. `desktop_elements` with the window: every interactive element with an id, role, name, value, states and window-relative extents. Add `query` to narrow ("Save", "entry", "tab").
2. `desktop_read` with the window: the readable text (headings, labels, links, field contents). Use it instead of reading screenshots.
3. `desktop_element_click` with the element id: activates it through the accessibility action, no pointer involved, works in every toolkit. `desktop_element_set_text` replaces an entry's or document's text the same way.
4. Only when an element has no action or the tree is missing (terminals, canvases, games): `desktop_screenshot`, then `desktop_click` / `desktop_type` / `desktop_key` **with `window` set**. Coordinates from `desktop_elements` are in the same space as window screenshots and `desktop_click`.

Every action returns a screenshot of the window taken right after it. Look at it: only report "clicked X" or "typed Y" if the screenshot shows the effect. If nothing changed, say so and try something else rather than claiming success.

Menus, dialogs and new windows appear as new elements: call `desktop_elements` again after opening one. A window that lists nothing has no accessibility support (or started before the accessibility bus); fall back to screenshots.

Tier: reversible. Closing a window may lose unsaved work: say so before closing an editor or browser with unsaved changes.

The SlateOS desktop's own shortcuts, for when the user asks: Super+Return terminal, Super+Space launcher, Super+s Slate prompt, Super+w browser, Super+e files, Super+t editor, Super+comma settings, Super+q close, Super+1..5 workspaces, Super+f fullscreen.
