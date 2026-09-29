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

## Commit messages

All new commits and PR titles must follow [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/):

```text
<type>[optional scope][!]: <description>
```

Use these project conventions:

- Write the subject in English, with a lowercase type and scope.
- Use a short, imperative description: `add`, `fix`, or `document`, rather than `added` or `fixed`. Do not end it with a period; aim for 72 characters or fewer.
- Choose a type from the table below. Keep each commit focused on one logical change.
- Use a scope when it clarifies the affected component: `slash`, `slated`, `slate`, `slate-desktop`, `slate-proto`, `desktop`, `distro`, `skills`, or `release`. Omit it for repository-wide changes; use the existing component names consistently.
- Add a body, separated by a blank line, when the motivation or implementation tradeoff needs explanation. Reference related issues or RFCs where relevant.

| Type | Use for |
| --- | --- |
| `feat` | New user-facing functionality. |
| `fix` | Bug fixes. |
| `docs` | Documentation changes. |
| `refactor` | Code restructuring without changing behavior. |
| `perf` | Performance improvements. |
| `test` | Test additions or corrections. |
| `build` | Build tooling, packaging, or dependency changes. |
| `ci` | CI workflows and automation. |
| `style` | Formatting-only changes, not desktop visual design. |
| `chore` | Maintenance that does not fit another type, including release preparation. |
| `revert` | Reverting an earlier change; identify the reverted commit in the body. |

Examples:

```text
feat(slash): add conversation history
fix(slate-desktop): verify focus before typing
docs: clarify snapshot recovery limits
build(distro): include desktop runtime dependencies
ci: check pull request titles
chore(release): prepare 0.0.12
```

These illustrate message format, not necessarily implemented features. Avoid vague subjects such as `update`, `fix stuff`, or `WIP`, and component-only prefixes such as `module:`.

For a breaking change, add `!` before the colon or a `BREAKING CHANGE:` footer. In this project, always explain the compatibility impact and migration steps in the body or footer, even during pre-alpha. For example, a hypothetical protocol change:

```text
feat(slash)!: rename the serve protocol prompt field

BREAKING CHANGE: JSON-lines clients must send `text` instead of `prompt`.
Update clients before upgrading slash.
```

## Pull requests

- One topic per PR.
- Use the same Conventional Commits format for the PR title. Describe the final change as a whole, and update the title if the scope changes.
- Fill out the [PR template](.github/PULL_REQUEST_TEMPLATE.md): explain what changed, why, how it was validated, and any compatibility or migration implications. The body uses normal prose and Markdown; it does not need a `feat:` or `fix:` prefix.
- Link the related issue or RFC. Use `Closes #123` only when the PR resolves that issue.
- Report the checks actually run and their results. If a relevant check was not run, say why; include screenshots for visible desktop changes.
- Design changes need an accepted RFC first. Implementation of an accepted design does not.

Before merging, check the PR title and commit subjects against these conventions. When squash-merging, use the PR title as the resulting commit subject and retain any breaking-change explanation in the commit body. These are review requirements; the current CI workflow does not enforce message formatting automatically.

## Code of conduct

See `CODE_OF_CONDUCT.md`.

## License

By contributing you agree your contribution is licensed under GPL-3.0-or-later.
