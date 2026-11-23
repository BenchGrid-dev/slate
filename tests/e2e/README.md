# End-to-end tests

These run against real components, not mocks. They need a machine with the Slate binaries, a wlroots desktop (sway) with `WAYLAND_DISPLAY` set, `foot` and `firefox`, and a logged-in Claude Code for the slash scenarios.

| Script | What it proves | Oracle |
|---|---|---|
| `desktop_e2e.py` | slate-desktop over MCP: launch, click, type (incl. Unicode), key combos, cropped screenshots, closing windows | files written by typed commands; window titles from the compositor |
| `slash_e2e.py` | slash + slated + Claude Code, or Codex with `--backend codex`: shell escape, context delta, memory, approvals (given and refused), undo | the agent's answers to factual questions; filesystem state |
| `serve_e2e.py` | `slash --serve`, the JSON protocol the desktop panel uses: events, approval round trip, /auto | event stream |
| `gateway_e2e.py` | An API connection of the person's own: slash, Claude Code, slash's gateway and a scripted OpenAI-format endpoint (`mock_openai.py`); the key stays out of the agent's environment; a refused key is reported at once | the endpoint's request log; event stream |

Run on the dev VM:

```
python3 tests/e2e/desktop_e2e.py ~/src/slate/target/debug/slate-desktop
python3 tests/e2e/slash_e2e.py ~/src/slate/target/debug/slash
SLATE_SNAPSHOT_ROOT=~/work python3 tests/e2e/slash_e2e.py ~/src/slate/target/debug/slash --backend codex --codex ~/root-codex/bin/codex
python3 tests/e2e/gateway_e2e.py ~/src/slate/target/debug/slash
```

With `SLATE_SNAPSHOT_ROOT` (a btrfs subvolume you own) `slash_e2e.py` works inside it and tests undo; its snapshots may need root to delete, and it prints the command. The Codex run needs a signed-in Codex and Slate's hooks in `/etc/codex/hooks.json`: on a system that does not run this tree's SlateOS, `target/debug/slate hook codex-config` prints the file to put there for the test.

`gateway_e2e.py` needs Claude Code but neither a sign-in nor a key (the endpoint is the mock), and also runs on macOS.

They are not in CI because they need a display and an agent subscription. Run them before every release.
