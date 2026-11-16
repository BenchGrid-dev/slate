# Releasing

Versions are small and frequent. A patch release is a snapshot the maintainer can test; a minor release (`0.1.0`, `0.2.0`) marks a milestone and is the maintainer's call.

0. Run `tests/e2e/` on the dev VM; do not release with failures.
1. Update `CHANGELOG.md` (move items from "Unreleased" under the new version with the date).
2. Bump `version` in the workspace `Cargo.toml` and in `flake.nix`; run `cargo build` so `Cargo.lock` follows.
3. Commit as `chore(release): prepare X.Y.Z` (see the [commit conventions](../CONTRIBUTING.md#commit-messages)) and push it.
4. Wait for CI. For a minor release (`X.Y.0`) the commit also starts the Image workflow, which builds the x86_64 image, installs it in a virtual machine, boots the result and updates it to a release made on the spot (about 40 minutes, see [tests/image](../tests/image/README.md)); do not tag before it passes. A patch release does not run it; start it by hand (Actions, Image, Run workflow) when the installer, the image or software update changed.
5. Tag `vX.Y.Z` and push the tag. The Release workflow starts (about half an hour). For both architectures it builds the image, checks that it holds no agent program (`distro/release/check-image.sh`: Claude Code is not SlateOS's to hand on, [decision 0010](decisions/0010-agent-programs-are-downloaded.md)), builds the reference installed systems, puts what cache.nixos.org lacks of them into SlateOS's binary cache, and attaches each image, in two parts with its checksum, to a draft release of the tag. When both are done it makes what software update reads and adds it to the draft: the source archive of the tag, the manifest (`slateos-update.json`: version, date, the changelog section's opening paragraph, the archive's checksum, the binary cache) and its signature by the release key. Wait for it.
6. Write the notes and publish the draft:

   ```
   gh release edit vX.Y.Z --draft=false --latest --notes-file NOTES
   ```

   Installed systems read `releases/latest/download/slateos-update.json`: a release published without the three files looks to every installation like a server that does not answer, so look for them on the draft first (`gh release view vX.Y.Z --json assets -q '.assets[].name'`). Afterwards, check with `slateos-update check` on an installed system of the version before.

   The notes end with how to get the image, since a release asset must stay under 2 GiB and the image does not:

   ```
   Download both parts of the image for your computer (x86_64: most PCs; aarch64: ARM, and virtual machines on Apple Silicon) and join them:

       cat slateos-X.Y.Z-x86_64-linux.iso.part1 slateos-X.Y.Z-x86_64-linux.iso.part2 > slateos-X.Y.Z-x86_64-linux.iso      # macOS, Linux
       copy /b slateos-X.Y.Z-x86_64-linux.iso.part1+slateos-X.Y.Z-x86_64-linux.iso.part2 slateos-X.Y.Z-x86_64-linux.iso    # Windows

   `sha256sum` (macOS: `shasum -a 256`, Windows: `certutil -hashfile FILE SHA256`) of the result is in slateos-X.Y.Z-x86_64-linux.iso.sha256.
   ```

## The release key

The manifest is signed with an Ed25519 key (`slateos-update keygen FILE`; `slateos-update public-key FILE` prints the half that goes into `services.slate.update.publicKeys`). Whoever holds it can sign an update that every installation accepts.

The Release workflow signs with it. The key is the secret `SLATEOS_RELEASE_KEY` of the repository's environment `release`, and that environment is limited to tags `v*`: a workflow run for a branch or a pull request cannot read it. One job reads it (`update`, the last one), which runs GitHub's checkout and the tag's own source and nothing else. `slateos-update verify` then checks the result against the keys the tag's module tells installed systems to trust, so a wrong key in the secret stops the release instead of reaching people.

This makes the repository the thing to protect: who can push a tag `v*` and publish a release can release SlateOS, with or without anyone's laptop. Keep that to people who may. To have every release wait for a person, add required reviewers to the environment (Settings, Environments, release).

Installed systems trust two keys: the one releases are signed with and a second one that is on no service (offline, with the maintainer). If the first is lost or no longer safe, a release signed with the second (`make-update.py X.Y.Z --key FILE`, then `gh release upload`) replaces it in `publicKeys`; without the second, installed systems could not be reached at all.

Setting it up, once (a fork does the same with its own keys):

```
gh api -X PUT repos/OWNER/REPO/environments/release --input - <<< '{"deployment_branch_policy": {"protected_branches": false, "custom_branch_policies": true}}'
gh api -X POST repos/OWNER/REPO/environments/release/deployment-branch-policies -f name='v*' -f type=tag
gh secret set SLATEOS_RELEASE_KEY --env release < ~/.config/slateos/release.key
```

`distro/release/make-update.py X.Y.Z --key FILE --out DIR` makes the same three files on any machine that has the key, for a release the workflow cannot make.

To change what an update can start from: `--from X.Y.Z` names the oldest version that can take this release directly, and `--step URL` the manifest older systems are sent to first.

## The binary cache

`distro/release/cache.json` names it: an address every installed system can read without an account, and the public half of the key its packages are signed with. The Release workflow fills it (`distro/release/fill-cache.sh`: the closure of the reference systems, less what cache.nixos.org has, less unfree packages and initrds) and uploads it (`upload-cache.sh`). It needs two repository secrets: `SLATEOS_CACHE_KEY`, the secret half of that key, and `AZURE_STORAGE_SAS`, a signature that lets it create blobs in the container and nothing else. `distro/release/azure-setup.sh` sets up the storage and both secrets once; the maintainer runs it. Without the secrets the workflow still builds and checks everything and uploads nothing.

The cache only grows. A release's packages must stay for as long as systems may update to it; older ones can be deleted from the container by hand. To move the cache, change `cache.json`: the next release's manifest names the new place, and installed systems follow it. Like the release key, the cache's key is a secret of the build service ([decision 0009](decisions/0009-software-update.md) says what that means).

Patch releases follow the NixOS release branch the flake pins (`nix flake update nixpkgs`, then the e2e suites), which is how security fixes of the packages reach installed systems.

The image is attached in two parts (it is larger than a release asset may be); it can also be built from the flake (`nix build github:BenchGrid-dev/slate#iso`), like the package alone (`nix build github:BenchGrid-dev/slate`, or `cargo build`). Installed systems update themselves from the release ([software update](software-update.md)).
