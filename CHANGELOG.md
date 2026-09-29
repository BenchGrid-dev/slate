# Changelog

Small releases, often. `0.0.x` are development snapshots; `0.1.0` comes after a full manual pass by the maintainer.

## Unreleased

## 0.0.17 — 2026-09-29

- Slate prompt: messages typed while Slate works are queued under the current exchange, each with edit, send-now (steer: the current turn is interrupted and the message follows) and remove; they go out one by one as turns end. `slash --serve` gains a `cancel` op that interrupts the running agent.
- Slate prompt: after sending, clicking another window puts the prompt away (it follows the person's focus); the colours follow `slate-theme`'s choice directly.
- Desktop: a dock at the bottom (pinned launcher, files, browser, terminal, settings, then the open windows; click to activate, middle-click to close) and an **AI** workspace left of 1 (Super+grave or Super+0): a live, read-only view of the agent's background screen. Logging in lands on workspace 1.

## 0.0.16 — 2026-09-29

- Fix: with the background screen in the layout, absolute input devices (tablets, touch, a virtual machine's pointer) mapped onto the whole layout and the cursor vanished past a fraction of the visible screen. The desktop daemon maps the person's pointer, touch and tablet devices to the region the real outputs cover.

## 0.0.15 — 2026-09-29

- Appearance: `slate-theme dark | light | wallpaper PATH` switches the whole desktop live (GTK/libadwaita through the colour-scheme key, sway colours and background, waybar, foot, mako, fuzzel, the Slate prompt) and remembers the choice; Settings → Appearance exposes it with a file chooser; the `appearance` skill lets the agent do it.
- Fix: the person's focus and cursor can no longer end up on the agent's background screen (the daemon pulls them back; a window mapped there at login is brought over).

## 0.0.14 — 2026-09-29

- The agent's own background screen: the SlateOS session runs sway with the headless backend and creates a second output nobody sees; `desktop_launch` opens windows there by default (`where: "background" | "here"`), `desktop_show` / `desktop_hide` move windows between the two screens (Super+b, panel middle click), `desktop_windows` reports each window's `location`, and the panel shows how many windows are in the background. The agent seat keeps one pointer per output. The desktop skill and slash's prompt say when to work where: results in the background, help with what the user is doing on their screen.

## 0.0.13 — 2026-09-28

- Live and installer image: `nix build .#iso` (`nixosConfigurations.iso-x86_64-linux` / `iso-aarch64-linux`) boots into the SlateOS desktop; `slateos-install --disk --user --host` partitions (EFI + btrfs with per-user home subvolumes), writes `/etc/slateos/configuration.nix` from a template with the full application set, and runs `nixos-install` from the image's store.
- Root with consent: `sudo` (and `ssh`) without a terminal ask the person through `slate-askpass`, a desktop dialog; the password goes to sudo only and each use asks again. The system skill and slash's prompt tell the agent to run plain `sudo`. System changes (`slateos-rebuild switch`, package installs) are now possible from a task.

## 0.0.12 — 2026-09-28

- slate-desktop: the accessibility tree as the first way to see and drive applications. `desktop_elements` lists a window's interactive elements (role, name, value, states, window-relative extents, actions), `desktop_read` returns its text, `desktop_element_click` activates an element through its accessibility action and `desktop_element_set_text` edits text over the bus; both work without a pointer or keyboard and in toolkits that ignore extra seats (GTK4). Pointer and typing fallbacks otherwise. Verified on GTK3 (Thunar, pavucontrol), GTK4 (settings app) and Firefox.
- Desktop profile: accessibility enabled for every toolkit (`services.gnome.at-spi2-core`, `toolkit-accessibility`, Qt/Firefox/Chromium switches); the daemon also turns the bus on at connect.
- Skills: `desktop-windows` teaches elements first, pixels second.
- Application suite: `services.slate.desktop.apps` (`full`, the default, or `minimal`). Full adds LibreOffice, Thunderbird, zathura and mpv; Mousepad and imv replace gnome-text-editor and loupe (GTK4, first-seat-only). New skills: `office-libreoffice`, `mail-thunderbird`, `pdf-zathura`, `media-mpv`, `text-mousepad`.

## 0.0.11 — 2026-09-28

- SlateOS system tools: `slateos-rebuild`, `slateos-option`, `slateos-install`, `slateos-generate-config`, `slateos-enter`, `slateos-version`, thin front ends for the `nixos-*` originals that use `/etc/slateos` (falling back to `/etc/nixos`). The system skill and slash's identity prompt use them. Licensing note in `distro/README.md` (nixpkgs is MIT; the NixOS name is used only descriptively).

## 0.0.10 — 2026-09-28

- Slate prompt: no more desktop freeze while Slate thinks. The takeover poll ran on the UI thread and could block for seconds behind a busy desktop daemon; with the overlay holding the keyboard that froze everything. The poll is off the main loop, and as soon as a prompt is sent the overlay drops to its panel-sized shape (no keyboard grab, no full-screen surface), so the agent's own clicks reach the apps and the user keeps their screen.
- Slate prompt: live activity while working: the model's reasoning tail when the backend streams it (new `thinking` event from Claude's `thinking_delta` and Codex `reasoning` items), the current tool, and an elapsed timer in the pill.
- Slate prompt: the card no longer draws a box-shadow (it was clipped to a hard rectangle on the panel-sized surface); MCP tool names are shown short.
- Settings: an **AI** page: backend (Claude Code / Codex), models, sign-in status with Sign in / Sign out (runs the official CLI in a terminal), verbose log and bypass-approvals defaults, written to `~/.config/slate/slash.toml`; a button restarts the prompt to apply.
- slash: `verbose` in `slash.toml` starts sessions in verbose mode.

## 0.0.9 — 2026-09-28

- SlateOS identity: os-release, boot entries, getty greeting and the default hostname say SlateOS; `ID_LIKE=nixos` is kept and skills match on it (`nixos-system` skill renamed `slateos-system`). slash introduces itself as the shell of SlateOS, built on NixOS.
- Slate prompt (slate-shell) redone in the spirit of Siri: a pill to type into and one exchange at a time below it; Esc or clicking elsewhere dismisses it; results and approvals reappear passively without taking the keyboard; a click claims it back.
- slate-desktop: `click` with `seat: "user"` pressed the button on the agent seat (wherever its pointer was) instead of the user's; motion and button now go through the same seat.
- slated: tasks started from the panel are `quiet` (no desktop notifications; the panel shows progress, approvals and results itself); the "Slate is working" notification is gone for everyone (the panel button shows it).
- Desktop theme: dark slate wallpaper (`distro/desktop/wallpaper.py`), Inter + JetBrains Mono + Font Awesome, translucent panel with icon glyphs, 1px window borders, dark GTK/libadwaita via dconf, foot/fuzzel/mako restyled.

## 0.0.8 — 2026-09-28

- Slate Shell: floating layer-shell panel (Mod+s / panel button) driving `slash --serve`: streamed answers, tool lines, approvals as buttons, status, takeover warning, Esc hides.
- `slash --serve`: JSON-lines session protocol for GUI clients.
- Takeover indicator: user-seat actions switch sway into a `controlling` mode and the panel blinks; Esc hands control back and blocks retries for a minute.
- e2e: serve protocol suite.
- Fixes: task records keep the user's prompt (not the context block); the panel status ignores tasks that died mid-turn.

## 0.0.7 — 2026-09-28

- Typing is focus-aware: `desktop_type` / `desktop_key` take a `window`, focus it for the seat in use (agent seat: title-bar click; user seat: compositor focus), verify via sway's per-seat focus, and report which window received the input. `desktop_seats` shows each seat's focus.
- `/auto on|off` in slash: bypass approvals for the session (Confirm-tier actions run, still audited and snapshotted); ⚡ prompt marker; `auto_approve` config.
- slash sets the terminal title (ready / working / approval needed); Slate notifications and the panel button focus the existing slash window instead of opening a new one.
- sway IPC: GET_SEATS is type 101.

## 0.0.6 — 2026-09-28

- Settings app (`slate-settings`, Mod+comma or right-click the panel's Slate button): display resolution, scale and a 2x HiDPI preset with confirm-or-revert; sound, network, Slate status and memories.
- `display-settings` skill: query, one change at a time, verify by screenshot, roll back, persist only to `~/.config/slate/sway.d/`.
- The desktop daemon is a supervised systemd user service started by sway (auto-restart, journal logs) with the session PATH; daemons log without panicking on a closed stderr (the "broken pipe" after launching Firefox).
- slated unit has btrfs and notify-send on PATH (snapshots had silently failed under systemd).
- e2e: geometry checks wait for windows to settle.

## 0.0.5 — 2026-09-28

- slate-desktop: a long-lived `daemon` owns the agent seat for the session (apps only accept input from seats present at their start); MCP and CLI proxy to it; sway profile starts it first.
- slate-desktop: click/type/key return a post-action screenshot; arrange positions are workspace-relative; `desktop_window_set` converges on the requested content geometry.
- slash: installed builds resolve companion binaries via PATH and warn when a newer Slate is installed; login shell is `/run/current-system/sw/bin/slash`.

## 0.0.4 — 2026-09-28

- slash: plain `exit` / `quit` / `退出` leaves slash; stdin is flushed before an approval prompt so stray keys cannot turn `y` into a denial.
- slated: command-aware dangerous-command detection (no more `op ` false positives on `$XDG_CURRENT_DESKTOP`); `/dev/null` redirects are not writes.
- slate-desktop: `desktop_close`, `desktop_focus`, `desktop_window_set` (move/resize/fullscreen), `desktop_arrange` (side_by_side / top_bottom / grid / maximize); workspace geometry from sway.
- Skills: `desktop-windows`.
- e2e: window arrangement, precise placement (self-correcting), and proper closing covered; the test terminal runs bash explicitly.
- Desktop profile: slash is the login shell for `loginShellUsers`; fallback shell defaults to bash.

## 0.0.3 — 2026-09-27

### slash
- Identity and behaviour in the system prompt: it is slash, the Slate shell; answers first; no unprompted repository chatter; lets Slate handle approvals.
- Fallback shell resolution never picks a path that does not exist (NixOS).

### slated
- Observe-tier calls that still reach the permission tool (AskUserQuestion) are allowed without asking.
- Desktop notifications for approvals and task start/end (notify-send); `slate agent-status` for panels.
- Memories deduplicate; instructions explain Slate memories vs Claude Code project memory.
- Protocol robustness: error replies carry the request id, clients time out, no duplicate `id` keys (a hang in `forget`).

### slate-desktop
- Window screenshots are cropped to the compositor's window geometry, so screenshot pixels match click coordinates (CSD shadow margins).
- ASCII text and key combos go through the standard US keymap with real keycodes (Firefox ignored text from made-up keycodes); the generated keymap is used only for non-ASCII.
- Settle after creating a seat so heavy clients bind it before input arrives.

### Desktop profile (phase 1)
- `services.slate.desktop.enable`: sway as a stacking desktop with title bars, waybar panel (launcher, workspaces, clock, volume, network, Slate status), fuzzel, mako, wallpaper, fonts and icons, Firefox, Thunar, text editor, greeter or autologin.

### Skills
- `slate skills install` only links skills whose requirements and distro match the machine.

### Tests
- `tests/e2e/`: desktop and slash end-to-end suites with real oracles (files written, window titles, agent answers). Both pass on the dev VM.

## 0.0.2 — 2026-09-27

Everything since the initial skeleton. Verified end to end on NixOS 26.05 / sway 1.12 with both Claude Code and Codex.

### slash
- Natural-language shell: bare text to the agent, `/` control commands, `!` runs in your real shell, `//` escapes a leading slash.
- `!` commands run in a pty: interactive programs work, output is captured for the agent, cwd and exported env persist.
- Claude Code backend (stream-json, session resume, streaming text) and Codex backend (`exec --json`, thread resume).
- Per-turn context: commands run since the previous turn (with output) and new memories travel inside the user message; static instructions in the system prompt.
- Default model `sonnet`; `/model`, `/agent`, `/new`, `/undo`, `/audit`, `/tasks`, `/remember`, `/memories`, `/context`, `/history`, `/cd`.
- Login-shell safe: non-interactive invocations exec the fallback shell; fallback resolution never picks a path that does not exist.

### slated
- Approval broker for Claude Code's permission tool; approvals answered at the terminal (`y` once, `a` for the task, `n` deny).
- Tier policy (Observe / Reversible / Confirm) with a shell classifier and `~/.config/slate/policy.toml` overrides.
- Append-only audit log; task store.
- Privilege-free btrfs snapshots per task, diff-based `undo` with preview, snapshot pruning.
- Memories: remember / recall / forget, deduplicated, injected into every task.

### slate CLI
- `hook pre-tool-use` / `post-tool-use`, `mcp` (approve, remember, recall, forget), `status`, `audit`, `tasks`, `undo`, `remember`, `memories`, `forget`, `skills install` / `list`.

### slate-desktop
- Agent seat on Wayland: transient seat, virtual pointer and keyboard, Unicode typing via generated keymaps, key combos.
- Per-window and full-output capture to PNG through ext-image-copy-capture.
- MCP tools: windows, screenshot, click, move, scroll, type, key, launch; window-relative coordinates from sway IPC.
- `seat: "user"` fallback for toolkits that bind only the first seat (GTK4), gated as Confirm.

### Distribution
- `flake.nix` with the package and a NixOS module (`services.slate.enable`, login shell users, sway, slated user service, snapshot root).
- Base OS Skills: audio, brightness, Wi-Fi, NixOS system, systemd, undo.

### Docs
- Architecture, roadmap, ADRs 0001–0007, policy file reference, contributing guide.

## 0.0.1 — 2026-09-26

Repository skeleton, architecture document, placeholder crates.
