---
name: slate-text-mousepad
description: Plain-text files on SlateOS: open a file for the user in Mousepad, insert or replace text, save. Use when the user wants to see or edit a text, config or code file in a window (for edits nobody needs to watch, edit the file directly).
---

# Text files in Mousepad

For an edit the user does not need to watch, edit the file directly (Read/Edit tools, `sed`); it is faster and exactly verifiable. Open Mousepad when the user wants the file on screen or wants to keep editing themselves.

- Open: `desktop_launch` with `mousepad /path/file.txt` (a new tab if Mousepad is already running).
- Mousepad (GTK3) has a full accessibility tree: the document is a `text` element with the file's contents as its value. `desktop_read` returns the text; `desktop_element_set_text` replaces the whole document; to insert, `desktop_element_click` the document, move with `desktop_key` (ctrl+End, ctrl+Home) and `desktop_type`.
- Save: `desktop_key` ctrl+s with the window set. A new file opens the Save As dialog: its name entry and Save button are elements.
- The window title shows `*` while there are unsaved changes; check it before closing.

Tier: reversible (files under the snapshot root are covered by undo). Confirm before overwriting a file the user did not name.
