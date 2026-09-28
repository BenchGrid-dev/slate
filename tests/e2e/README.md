# End-to-end tests

These run against real components, not mocks. They need a machine with the Slate binaries, a wlroots desktop (sway) with `WAYLAND_DISPLAY` set, `foot` and `firefox`, and a logged-in Claude Code for the slash scenarios.

| Script | What it proves | Oracle |
|---|---|---|
| `desktop_e2e.py` | slate-desktop over MCP: launch, click, type (incl. Unicode), key combos, cropped screenshots, closing windows | files written by typed commands; window titles from the compositor |
| `slash_e2e.py` | slash + slated + Claude Code: shell escape, context delta, memory, approvals, undo | the agent's answers to factual questions; filesystem state |
| `serve_e2e.py` | `slash --serve`, the JSON protocol the desktop panel uses: events, approval round trip, /auto | event stream |

Run on the dev VM:

```
python3 tests/e2e/desktop_e2e.py ~/src/slate/target/debug/slate-desktop
python3 tests/e2e/slash_e2e.py ~/src/slate/target/debug/slash
```

They are not in CI because they need a display and an agent subscription. Run them before every release.
