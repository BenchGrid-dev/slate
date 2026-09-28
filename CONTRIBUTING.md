# Contributing to Slate

SlateOS runs end to end on the maintainer's machine; what it needs now is more hardware, more apps and more people. The most valuable contributions are reports from running it, OS Skills, compositor and accessibility work, and answers to the open questions in `docs/architecture.md`.

## Where to start

1. Read `README.md`, then `docs/architecture.md` and `docs/roadmap.md`. The "Open questions" section lists what is undecided; the roadmap lists what is next.
2. Look at issues labelled `good first issue`, `help wanted` or `rfc`.
3. Say hello in an issue before starting anything large, so two people do not build the same thing.

## Kinds of contribution

- **RFCs.** Design proposals for open questions. See `docs/rfcs/README.md`.
- **Reports.** Run it (see `distro/README.md`) and say what happened: GPU, scale, which apps the agent seat reached, what broke. Screenshots help.
- **Prototypes.** Small, throwaway programs that prove or disprove a mechanism (for example: "does Qt bind a second seat"). Put them under `prototypes/<name>/` with a README saying what was learned. They do not need to be pretty.
- **Code.** Rust, in the workspace. `cargo fmt`, `cargo clippy -- -D warnings` and `cargo test` must pass; CI enforces this. Anything touching slash, slated or slate-desktop should also pass `tests/e2e/` on a real desktop (see `tests/e2e/README.md`).
- **OS Skills.** See `skills/README.md`. These need no Rust at all.
- **Docs.** Both `README.md` and `README.zh-CN.md` should be kept in sync; if you only speak one language, update that one and say so in the PR.

## Pull requests

- One topic per PR.
- Describe what and why. Link the issue or RFC.
- Design changes need an accepted RFC first. Implementation of an accepted design does not.

## Code of conduct

See `CODE_OF_CONDUCT.md`.

## License

By contributing you agree your contribution is licensed under GPL-3.0-or-later.
