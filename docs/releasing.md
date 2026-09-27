# Releasing

Versions are small and frequent. Every `0.0.x` is a snapshot the maintainer can test; `0.1.0` is cut only after a full manual pass.

0. Run `tests/e2e/` on the dev VM; do not release with failures.
1. Update `CHANGELOG.md` (move items from "Unreleased" under the new version with the date).
2. Bump `version` in the workspace `Cargo.toml` and in `flake.nix`; run `cargo build` so `Cargo.lock` follows.
3. Commit as `Release X.Y.Z`, tag `vX.Y.Z`, push both.
4. `gh release create vX.Y.Z --notes-from-tag` (or paste the changelog section).

Binaries are not attached yet; installs go through the flake (`nix build github:BenchGrid-dev/slate`) or `cargo build`.
