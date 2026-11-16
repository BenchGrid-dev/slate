# 0009: Software update: a release replaces the source, the system is built again and started once on trial

- Status: accepted
- Date: 2026-11-15

## Context

An installed SlateOS did not move. Its configuration is `/etc/slateos/configuration.nix`, built from a copy of the Slate source next to it; its packages came from whatever the image's channel held. A newer release reached it only when someone replaced that source by hand and rebuilt, and the NixOS packages underneath followed a channel nobody updated, or one that had moved on to revisions no SlateOS release was tested with.

People expect what phones and macOS do: the system says there is a new version, installs it while they work, and a restart finishes it. And they expect not to be left with a computer that does not start.

Three things are particular to SlateOS:

- Every installation differs. The person's name, the host name, the disk layout, the applications and input methods they chose are part of the system that is built. A release cannot ship one image for all.
- NixOS already keeps every earlier system as a generation the boot menu can start. What it lacks is choosing between them without a person at the boot menu.
- NixOS 26.05's systemd-boot module has no boot counting.

## Decision

**A release is the Slate source at a tag, and it fixes the whole system.** The flake lock names the nixpkgs revision the release was tested on. An installed system is evaluated against that revision, by the updater and by `slateos-rebuild` alike, and no longer against a channel.

**An update replaces the source and builds the configuration again.** `slateos-update` downloads the release's source, puts it in place of `/etc/slateos/slate` (keeping the old one), evaluates the person's configuration against the release's nixpkgs and builds it. Prebuilt packages come from cache.nixos.org and from a binary cache of SlateOS's own for what that one lacks; what carries the machine's names is built on the machine. This is the same work an installation does. The manifest names that cache and its key, so where a release's packages are kept is the release's business and can change.

**The release is named by a signed manifest.** A small JSON file on the release page gives the version, the source archive's address and its SHA-256. It is signed with Ed25519; installed systems hold the public keys and read nothing else. The build service signs it when a release is tagged: the key is a secret that only a run for a release tag can read. A second key, which installed systems trust as well, is kept off every service.

**A new version is started once before it becomes the default.** The new system is entered in the boot menu while the running one stays the default, and systemd-boot is told to start the new entry at the next start only (its one-shot variable). When the new system has finished starting and its desktop session has stayed up, it makes itself the default. If the old version starts instead, the new one did not come up: the old version removes it and puts the source and the boot menu back.

**The person starts the installation and the restart.** By default SlateOS only checks and notifies. Downloading unasked is a choice in Settings; restarting unasked is not offered.

**The updater of the release being installed finishes the installation.** Once the new system is built, its own `slateos-update` enters it in the boot menu. A fault in that step can then be mended by a release, instead of staying in every system that has not yet updated.

## Consequences

- A SlateOS version means one set of packages on every computer that runs it, which is what the tests of a release vouch for. Security fixes of NixOS packages reach people through SlateOS releases, so patch releases have to follow the NixOS branch regularly.
- An update costs evaluation and some building on the person's machine: a minute or two for a release that only changes Slate, more when nixpkgs moves. Without SlateOS's binary cache it also compiles Slate and the patched compositor there.
- The trial needs systemd-boot and firmware that keeps its variables. Where either is missing the new version becomes the default at once, and recovery is the boot menu, as on any NixOS. The updater sees firmware that forgets variables from the boot entry the installation wrote, and does not promise a trial there.
- "Came up" is judged by the session of the person the desktop logs in by itself. A system with a login screen counts as up when it has finished starting.
- Between the installation and the restart, the source on disk is already the new release's. `slateos-rebuild` refuses to switch in that window, because what it built would be the new release and the default, past the trial.
- The binary cache is filled and signed by the build service, for two architectures no single machine of the maintainer's builds. Its key is therefore a secret of that service, and whoever controls the service could put a wrong binary under a right name. The manifest's signature does not cover that. It is the trust every distribution places in its build farm; a system that will not place it turns the release's cache off and compiles.
- The release key decides what every installation runs, and it is where the releases are made: whoever can tag and publish a release in the repository can release SlateOS. The signature guards the way from the release to the computer (the server the files lie on, the network); it does not guard against the repository itself being taken over. The binary cache already rests on the same service.
- Two public keys ship: the one releases are signed with and a second kept offline, so that a first key that is lost or no longer safe can be replaced by a release signed with the second.
- Systems installed before this (0.2) have no updater and take one manual step to 0.3.0.
- The manifest can name a release older systems must take first (`from`, `step`), so a later change to this mechanism can go through an intermediate release.

## Alternatives considered

- **Image-based updates with two system partitions** (systemd-sysupdate, as on ChromeOS or Fedora's image mode). Atomic and fast, and the same bits on every machine. It does not fit a system whose configuration is the person's own file: user names, applications and input methods would have to move out of the system into a layer on top. Worth another look if SlateOS stops being configured through NixOS.
- **`system.autoUpgrade` on a channel or a flake reference.** Already in NixOS. It follows a branch instead of releases, switches or restarts on a timer, has no notion of a version that did not start, and nothing to show a person.
- **A hosted service for flake releases** (FlakeHub and the like). Paid, and built around fixed configurations.
- **systemd's boot counting.** The right mechanism for the trial, with a counter instead of a single try. It would mean carrying a patch to NixOS's boot loader installer; the one-shot entry needs none, and one try is what a person at the power button does anyway.
- **A release key on the maintainer's machine only.** Then taking over the repository would not be enough to release. But every release would wait for that one machine and a step done by hand, and the binary cache, which the same installations trust, is signed by the build service anyway. The offline second key keeps what matters of this: a way back that no service holds.
- **Building the release's system for the person** and shipping the closure. Impossible in general for the reason above; the binary cache gives most of the speed without it.
