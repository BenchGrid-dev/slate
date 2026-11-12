# Releasing

Versions are small and frequent. A patch release is a snapshot the maintainer can test; a minor release (`0.1.0`, `0.2.0`) marks a milestone and is the maintainer's call.

0. Run `tests/e2e/` on the dev VM; do not release with failures.
1. Update `CHANGELOG.md` (move items from "Unreleased" under the new version with the date).
2. Bump `version` in the workspace `Cargo.toml` and in `flake.nix`; run `cargo build` so `Cargo.lock` follows.
3. Commit as `chore(release): prepare X.Y.Z` (see the [commit conventions](../CONTRIBUTING.md#commit-messages)) and push it.
4. Wait for CI. For a minor release (`X.Y.0`) the commit also starts the Image workflow, which builds the x86_64 image, installs it in a virtual machine and boots the result (about an hour, see [tests/image](../tests/image/README.md)); do not tag before it passes. A patch release does not run it; start it by hand (Actions, Image, Run workflow) when the installer or the image changed.
5. Tag `vX.Y.Z`, push the tag, and `gh release create vX.Y.Z --notes-file` with the changelog section.

Patch releases follow the NixOS release branch the flake pins (`nix flake update nixpkgs`, then the e2e suites), which is how security fixes of the packages reach installed systems.

Binaries and the image are not attached yet (the image is larger than a release asset may be); installs go through the flake (`nix build github:BenchGrid-dev/slate`, `nix build github:BenchGrid-dev/slate#iso`) or `cargo build`.
