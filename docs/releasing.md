# Releasing

Versions are small and frequent. A patch release is a snapshot the maintainer can test; a minor release (`0.1.0`, `0.2.0`) marks a milestone and is the maintainer's call.

0. Run `tests/e2e/` on the dev VM; do not release with failures.
1. Update `CHANGELOG.md` (move items from "Unreleased" under the new version with the date).
2. Bump `version` in the workspace `Cargo.toml` and in `flake.nix`; run `cargo build` so `Cargo.lock` follows.
3. Commit as `chore(release): prepare X.Y.Z` (see the [commit conventions](../CONTRIBUTING.md#commit-messages)) and push it.
4. Wait for CI. For a minor release (`X.Y.0`) the commit also starts the Image workflow, which builds the x86_64 image, installs it in a virtual machine, boots the result and updates it to a release made on the spot (about 40 minutes, see [tests/image](../tests/image/README.md)); do not tag before it passes. A patch release does not run it; start it by hand (Actions, Image, Run workflow) when the installer, the image or software update changed.
5. Tag `vX.Y.Z` and push the tag.
6. Make what software update reads, and publish the release with it attached:

   ```
   distro/release/make-update.py X.Y.Z --key ~/.config/slateos/release.key --out release-X.Y.Z
   gh release create vX.Y.Z --title "SlateOS X.Y.Z" --notes-file NOTES release-X.Y.Z/*
   ```

   `make-update.py` writes the source archive of the tag, the manifest (`slateos-update.json`: version, date, the changelog section's opening paragraph, the archive's checksum) and its signature. Installed systems read `releases/latest/download/slateos-update.json`, so the three files go up with the release, not after it: a release without them looks to every installation like a server that does not answer. Check with `slateos-update check` on an installed system of the version before.

## The release key

The manifest is signed with an Ed25519 key (`slateos-update keygen FILE`; `slateos-update public-key FILE` prints the half that goes into `services.slate.update.publicKeys`). Whoever holds it can sign an update that every installation accepts, so it stays on the maintainer's machine, readable by them alone, and is not given to the build service. Installed systems trust two keys: the one releases are signed with and a second one kept somewhere else (offline). If the first is lost or no longer safe, a release signed with the second replaces it in `publicKeys`; without the second, installed systems could not be reached at all.

To change what an update can start from: `--from X.Y.Z` names the oldest version that can take this release directly, and `--step URL` the manifest older systems are sent to first.

Patch releases follow the NixOS release branch the flake pins (`nix flake update nixpkgs`, then the e2e suites), which is how security fixes of the packages reach installed systems.

Binaries and the image are not attached yet (the image is larger than a release asset may be); installs go through the flake (`nix build github:BenchGrid-dev/slate`, `nix build github:BenchGrid-dev/slate#iso`) or `cargo build`. Installed systems update themselves from the release ([software update](software-update.md)).
