# Image test

`install_test.py` installs SlateOS from its image in QEMU and boots what was installed. It is the test of the path a new user takes, so it changes nothing about the image: no test hooks, no special boot options.

| Step | What happens | Oracle |
|---|---|---|
| Boot the image | Firmware (UEFI), the boot menu's default entry, the live system | a login prompt on the live system's console |
| Install | `slateos-install` on an empty disk, with `--passphrase-file` for an encrypted one | its exit status |
| Boot the disk | Without the image; the disk's passphrase is typed on the keyboard once the screen stands still | a login prompt on the installed system's console |
| Check | The new user logs in (their login shell is slash; `!` runs the checks) | `slate-desktop`, `slate-dock`, `slate-panel` and `slated` are active, sway runs, `slateos-version` names the image's version, the root is btrfs (inside LUKS when encrypted) |

The console is a virtio console (`hvc0`), which systemd gives a login prompt on any system; the screen and the keyboard stay what a person has. Logs and screenshots (`desktop.png`, `passphrase-prompt-*.png`, `failure.png`) are written to `--workdir`.

```
nix build .#iso
nix shell nixpkgs#qemu_kvm -c python3 tests/image/install_test.py \
  --iso result/iso/slateos-*.iso \
  --firmware "$(nix build nixpkgs#OVMF.fd --no-link --print-out-paths)/FV/OVMF.fd" \
  [--encrypt]
```

On aarch64 the firmware is `FV/QEMU_EFI.fd` of the same package. With `/dev/kvm` one run takes about fifteen minutes; without it QEMU emulates the processor, every wait is multiplied by eight (`--patience`) and a run takes more than an hour. The disk needs about 10 GB in `--workdir`; a full host disk pauses the guest, which the test reports instead of waiting.

CI runs it on x86_64 for minor releases and on request (`.github/workflows/image.yml`, see [releasing](../../docs/releasing.md)). The aarch64 image is run by hand on the development system, under emulation.
