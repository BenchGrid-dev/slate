# Software update

A system the SlateOS installer set up updates itself to newer releases: Settings, Software Update, or `slateos-update` in a terminal. It looks for a new version once a day and says so; installing is yours to start, and so is the restart that finishes it.

| | |
|---|---|
| **What an update is** | The next SlateOS release, whole: Slate's programs, the desktop, and the NixOS packages that release was tested on |
| **What it leaves alone** | Your files, your settings, the applications you installed from Flathub, what you added to `/etc/slateos/configuration.nix` |
| **When it takes effect** | At the next restart. Until then the version you run keeps running |
| **If the new version does not start** | The restart after that is the old version again, by itself, with everything as it was |
| **If you do not like it** | Go Back, in Settings, returns to the version before |

A system that uses the SlateOS module in a NixOS configuration of its own (a flake, a channel) is not updated this way: it updates that configuration, as before.

## Updating

**Settings, Software Update** shows the version that runs and, when there is one, the newer version with a few words on what changed. **Update Now** asks for your password and installs it in the background; you can keep working. When it says the version is ready, **Restart Now** starts it. **Take Back** undoes the installation before the restart.

From a terminal:

```sh
slateos-update                # the version that runs, and what is known about a newer one
slateos-update check          # ask the release server now
sudo slateos-update install   # download and build the newest release
sudo slateos-update cancel    # take it back before the restart
sudo slateos-update back      # make the version before the last update the default again
```

An update needs a network and, for a release that moves to newer NixOS packages, a few gigabytes of free space and some minutes. It tells you how much it downloads before it starts.

## What happens without you asking

Settings, Software Update, **Automatic Updates** (changing it needs your password):

| Choice | What SlateOS does |
|---|---|
| Check for updates (the default) | Asks the release server once a day and a while after each start, and shows a notification when there is a new version |
| Download and prepare updates | Also installs the new version in the background, while the computer is plugged in. Only the restart is left to you |
| Neither | Nothing; Check Now still works |

Nothing restarts the computer by itself. A version that failed to install, or that you went back from, is not tried again unasked.

## The trial start

A new version is not made the default when it is installed. The boot loader is told to start it once; the version you run stays the default. After the restart SlateOS waits until the system has finished starting and the desktop session has stayed up for a while. Only then does the new version become the default.

If it does not get that far (it hangs, the screen stays black, the desktop does not come up), turn the computer off and on. The old version starts, sees that the new one did not come up, and removes it: the Slate source, the boot menu and the system's generations are as they were before the update. Settings says what happened. You can install the version again when you want to; SlateOS does not try by itself.

The trial needs systemd-boot (what the installer sets up) and firmware that keeps the boot loader's variables across a restart, as computers and most virtual machines do. Where it is not possible (`services.slate.update.trial = false`, another boot loader, QEMU without a variable store), a new version is the default at once, and one that does not start is left by choosing the older entry in the boot menu; that start then puts everything back the same way.

## How a release reaches your computer

- A release is the Slate source at a tag. Its `flake.lock` names the nixpkgs revision the release was built and tested on, so a SlateOS version is one fixed set of packages, not "Slate plus whatever NixOS has today".
- Each release publishes a small manifest (`slateos-update.json`): the version, the date, a summary, where the source archive is and its SHA-256. The manifest is signed with SlateOS's release key (Ed25519). Your system holds the public half (`services.slate.update.publicKeys`) and reads no manifest that is not signed with it; the source archive is checked against the manifest's checksum. So neither the server the files come from nor the network in between can make your computer install something else.
- A version is never replaced by an older one this way.
- The installed system keeps the Slate source next to its configuration, in `/etc/slateos/slate`. An update puts the new source there (the old one is kept as `slate.previous`), evaluates `/etc/slateos/configuration.nix` against the release's nixpkgs, and builds the system. What cache.nixos.org holds is downloaded, and so is what the release's own binary cache holds: Slate itself and the packages SlateOS patches, built for both architectures when the release was made. The rest, mostly the small files with your computer's names in them, is built on your machine. Claude Code, where the system has it, comes from Anthropic's download server in the version the release names: no cache of SlateOS's holds it ([connecting the agent](agent-connection.md)).
- The release's cache is named in the signed manifest, with the key its packages are signed with, so it can move without installed systems having to learn a new address. It is filled and signed by SlateOS's build service, not on the maintainer's machine as the manifest is. A system that would rather compile than rely on that sets `services.slate.update.releaseCache = false`; the update then takes longer and rests on the release key alone.
- `slateos-rebuild` evaluates against the same nixpkgs, so a rebuild between updates (an input method from Settings, a package you added) does not move the system to other package versions. `nix-shell -p` and `nix shell nixpkgs#…` use that revision too.

The state is in `/var/lib/slateos-update/state.json`, readable by everyone; `log` next to it holds the last installation's output.

## For other releases than SlateOS's

A fork, or a company with its own releases, points the updater at them in the configuration:

```nix
services.slate.update = {
  url = "https://example.org/releases/slateos-update.json";
  publicKeys = [ "…base64…" ];                 # slateos-update keygen FILE prints it
};
```

`distro/release/make-update.py` writes a release's three files, the manifest with the binary cache it names (`distro/release/cache.json`); [releasing](releasing.md) has the steps. `services.slate.update.cache` adds a cache the system always asks.

## Limits

- Only systems laid out by the installer (0.2 and later) are updated this way. One installed from a 0.2 image has no updater yet; it gets there once, by hand: replace `/etc/slateos/slate` with the 0.3.0 source and run `sudo slateos-rebuild switch`.
- The check and the download need a network; there is no update from a USB stick with a button, though `slateos-update --config` can read a manifest from a `file://` address.
- An update that changes the kernel or the desktop takes effect at the restart, never in the running session.
- What you changed inside `/etc/slateos/slate` is replaced (and kept in `slate.previous` until the update after).

How this is built, and why: [decision 0009](decisions/0009-software-update.md).
