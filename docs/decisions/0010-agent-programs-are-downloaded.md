# 0010: The agent's programs are downloaded by each computer, not shipped on the image

- Status: accepted
- Date: 2026-11-15

## Context

SlateOS's agent is a vendor's program: Claude Code, or Codex (0001). Up to 0.2.2 the image carried Claude Code, so an installation without a network still ended with an agent to sign in to, and the installer's sign-in ran in the live session.

Claude Code is not free software. nixpkgs marks it unfree, its binary is published by Anthropic on Anthropic's server, and nothing in its terms lets a third party pass copies on. While the image was a file on a developer's disk that did not matter. With 0.3 the image is attached to a public release, for anyone to download: SlateOS would be distributing Claude Code.

The agent is also what SlateOS is for. A system without one is a desktop that does not answer, which is why 0.2.2 made connecting it a step the installer does not go past.

## Decision

The image holds no agent program. It names them: which version this release was tested with and where each comes from (`/etc/slateos/agents.json`, from the same pinned nixpkgs as the rest of the system). Every computer downloads them itself, from the place their makers publish them: Claude Code from Anthropic's download server, as nixpkgs' package does, and Codex, which is free software, from cache.nixos.org.

- **Installing.** `slateos-install` writes the programs into the new system's `settings.json` and the installation downloads them to the disk. It asks each source first whether it answers, and leaves out what cannot be reached; when the installation with the programs fails, it runs again without them. An installation never fails for want of an agent program.
- **The live session** gets a program only when something there needs it: a sign-in in the installer, or someone trying the agent before installing. `slateos-live-agents` downloads it into the live store, which is memory, and the installation copies it from there. What putting Claude Code together takes besides its binary (the tools of nixpkgs' recipe) is on the image, so that the binary is the one thing that comes over the network. The recipe is made in the live session, from the image's nixpkgs: a recipe in an image's store would make Nix want everything it builds on the image too.
- **An installed system** that lacks them offers the download where the agent is set up: the welcome at first login, and Settings, AI. It is `slateos-settings set agents`, which writes `agents` in `settings.json` and rebuilds, the way input methods are installed.
- **The step stays required.** The installer's Continue still waits for a connection. Skip is new, for a computer without a network or a person without an account yet; it says first what a system without an agent cannot do, and the welcome returns at every login until the agent works.
- **A check keeps it so.** The Release and Image workflows fail when the image's store contains a program it names (`distro/release/check-image.sh`), and the image test asks the live system the same.

Both programs are downloaded, although Codex could legally be on the image. One rule for both is simpler to explain and to check; and with Codex alone on the image, the agent an installation without a network ends with would be the one without SlateOS's approvals and undo.

## Consequences

- SlateOS distributes no vendor's program. Each person's computer fetches Claude Code from Anthropic, as it would with Anthropic's own installer.
- Installing with an agent needs the internet, and a few hundred megabytes more of it. An installation without a network gives a system that asks for the download at its first login; "Minimal, no internet needed" still installs, to a desktop without an agent.
- An API key is complete in the installer without Claude Code, because the connection test is Slate's own code. A subscription's sign-in needs its program in the live session first: one more download, held in memory (about 300 MB for Claude Code, 500 MB for Codex). `slateos-live-agents` refuses when memory is short and says to install without.
- Where Anthropic's download server cannot be reached, Claude Code cannot be installed, also for someone whose model is served elsewhere. The installation then goes on with what it can get.
- The installed system's programs are part of its closure and of the release's pin, as before: a release brings the versions it was tested with, software update downloads them the same way, and going back to an older version brings the older programs back.
- Existing configurations keep Claude Code: without `agents` in `settings.json` the default of `services.slate.agents` is unchanged.
- A ChatGPT subscription is a choice in the form now, since Codex is downloaded beside Claude Code. It makes Codex the agent, with what 0008 says about it: no approvals, no undo.

## Alternatives considered

- **Keep Claude Code on the image.** The simplest for the person, and distribution without a licence.
- **Ask Anthropic for permission.** Possible later; a release cannot wait on it, and the design above works either way.
- **The vendors' own installers** (`claude.ai/install.sh`, npm) into the person's home. Always the newest version and self-updating, but outside the release's pin: the agent would change under a system that was tested with another, a NixOS without `nix-ld` cannot run Anthropic's binary as downloaded, and going back to an older SlateOS would not bring the older agent back.
- **Download everything into the live session before installing.** One mechanism instead of two, and close to a gigabyte of memory on the computers least able to spare it. The installation writes to the disk anyway.
- **Sign in at the first start instead of in the installer.** No download in the live session, but the installer could no longer tell whether the agent will work, which is what its step is for.
