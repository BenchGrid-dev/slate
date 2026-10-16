# Releasing

Versions are small and frequent. A patch release is a snapshot the maintainer can test; a minor release (`0.1.0`, `0.2.0`) marks a milestone and is the maintainer's call.

0. Run `tests/e2e/` on the dev VM; do not release with failures.
1. Update `CHANGELOG.md` (move items from "Unreleased" under the new version with the date).
2. Bump `version` in the workspace `Cargo.toml` and in `flake.nix`; run `cargo build` so `Cargo.lock` follows.
3. Commit as `chore(release): prepare X.Y.Z` (see the [commit conventions](../CONTRIBUTING.md#commit-messages)), tag `vX.Y.Z`, push both.
4. `gh release create vX.Y.Z --notes-from-tag` (or paste the changelog section).

Binaries are not attached yet; installs go through the flake (`nix build github:BenchGrid-dev/slate`) or `cargo build`.
