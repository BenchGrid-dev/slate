# Contributing to Slate

Slate is in its design phase. The most valuable contributions right now are design arguments, prototypes that settle open questions, and OS Skills.

## Where to start

1. Read `README.md`, then `docs/architecture.md`. The "Open questions" section lists what is undecided.
2. Look at issues labelled `good first issue`, `help wanted` or `rfc`.
3. Say hello in an issue before starting anything large, so two people do not build the same thing.

## Kinds of contribution

- **RFCs.** Design proposals for open questions. See `docs/rfcs/README.md`.
- **Prototypes.** Small, throwaway programs that prove or disprove a mechanism (for example: "can a transient seat drive Firefox on sway"). Put them under `prototypes/<name>/` with a README saying what was learned. They do not need to be pretty.
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
