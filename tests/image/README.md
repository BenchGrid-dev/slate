# Image test

`install_test.py` installs SlateOS from its image in QEMU and boots what was installed. It is the test of the path a new user takes, so it changes nothing about the image: no test hooks, no special boot options.

| Step | What happens | Oracle |
|---|---|---|
| Boot the image | Firmware (UEFI), the boot menu's default entry, the live system | a login prompt on the live system's console; the image names the agent programs and holds neither (`slateos-live-agents --status`, the store's listing) |
| Claude Code in the live session (`--live-agent`) | `slateos-live-agents claude-code`, as a sign-in in the installer does; then the network is turned off | `claude --version` answers in the live session, and nothing answers from outside |
| Install | `slateos-install` on an empty disk, with `--passphrase-file` for an encrypted one, `--home-files` with an agent connection, and with `--offline` no network card (the minimal application set promises to need none) | its exit status, and the agent programs it says it installed: none without a network, Claude Code alone and copied (not built again) after `--live-agent`, both otherwise |
| Boot the disk | Without the image; the disk's passphrase is typed on the keyboard once the screen stands still | a login prompt on the installed system's console |
| Check | The new user logs in (their login shell is slash; `!` runs the checks) | `slate-desktop`, `slate-dock`, `slate-panel` and `slated` are active, sway runs, `slateos-version` names the image's version, the root is btrfs (inside LUKS when encrypted), the agent's connection is in the new home, private and the user's, `/etc/slateos/settings.json` names the agent programs the installation reported, and those answer `--version` (and no other is on PATH) |
| The missing program (`--live-agent`) | `sudo slateos-settings set agents all`, which is what the welcome and Settings run: the system was installed with Claude Code alone and now has a network | `settings.json` names both programs, and both answer `--version` |
| Update (`--update REPO`) | A release one version newer is made from the repository: its Slate package built and put in a binary cache, its source and signed manifest served to the guest. `slateos-update install` in the guest, then a restart | `slateos-update` exits 0 and has set up a trial start |
| A version that does not start | The new version's boot entry gets `systemd.unit=emergency.target`; no login prompt for 100 s; the machine is reset | the version from before comes up, `slateos-update` says `rolled-back`, and the new version's generation, boot entry and source are gone |
| A version that starts | The same release installed again, left alone, restarted | `slateos-version` names the new version; once the session stays up `slateos-update` says `installed` and the boot menu's default is the new generation |
| The way back | `slateos-update back`, restart | the version from before runs, `slateos-update` says `went-back` |

The console is a virtio console (`hvc0`), which systemd gives a login prompt on any system; the screen and the keyboard stay what a person has. Logs and screenshots (`desktop.png`, `passphrase-prompt-*.png`, `failure.png`) are written to `--workdir`.

```
nix build .#iso
nix shell nixpkgs#qemu_kvm -c python3 tests/image/install_test.py \
  --iso result/iso/slateos-*.iso \
  --firmware "$(nix build nixpkgs#OVMF.fd --no-link --print-out-paths)/FV/OVMF.fd" \
  [--encrypt]
```

On aarch64 the firmware is `FV/QEMU_EFI.fd` of the same package. The update needs firmware that keeps its variables across a restart, as computers do (the trial start is one): `--firmware FV/OVMF_CODE.fd --firmware-vars FV/OVMF_VARS.fd --update .`, plus `nix` and `git` on the host, whose architecture must be the image's. With `/dev/kvm` one run takes about five minutes, the update a quarter of an hour more; without it QEMU emulates the processor, every wait is multiplied by eight (`--patience`) and a run takes more than an hour. The disk needs about 10 GB in `--workdir`; a full host disk pauses the guest, which the test reports instead of waiting.

CI runs it on x86_64 for minor releases and on request (`.github/workflows/image.yml`, see [releasing](../../docs/releasing.md)). The aarch64 image is run by hand on the development system, under emulation.
