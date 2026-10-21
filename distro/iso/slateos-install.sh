# slateos-install: put SlateOS on a disk. Run from the live image as root.
#
#   sudo slateos-install --disk /dev/nvme0n1 --user alice --host mybox [--tz Europe/Berlin]
#   --encrypt              encrypt the disk (LUKS2); its passphrase is asked at every boot
#   --apps minimal|full    the application set (default full; minimal for small disks)
#   options for scripted use: --yes, --password-hash HASH, --passphrase-file FILE, --dry-run
#
# Layout: GPT, 512 MiB EFI system partition, the rest btrfs (inside LUKS2 with --encrypt)
# with subvolumes @ (root), @nix, @home, and @home/<user> owned by the user, which is
# what Slate's snapshots and undo need. Everything on the disk is erased.
set -euo pipefail

disk="" user="" host="slateos" tz="UTC" dry=0 yes=0 hash="" encrypt=0 passfile="" apps="full"
while [ $# -gt 0 ]; do
  case "$1" in
    --disk) disk="$2"; shift 2 ;;
    --user) user="$2"; shift 2 ;;
    --host) host="$2"; shift 2 ;;
    --tz) tz="$2"; shift 2 ;;
    --apps) apps="$2"; shift 2 ;;
    --encrypt) encrypt=1; shift ;;
    --passphrase-file) encrypt=1; passfile="$2"; shift 2 ;;  # scripted; otherwise asked
    --password-hash) hash="$2"; shift 2 ;;   # for scripted installs; otherwise asked
    --yes) yes=1; shift ;;                   # skip the disk confirmation (scripted installs)
    --dry-run) dry=1; shift ;;               # partition, subvolumes and configuration only
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$disk" ] && [ -n "$user" ] || { echo "usage: slateos-install --disk DEV --user NAME [--host NAME] [--tz ZONE] [--encrypt] [--apps minimal|full]" >&2; exit 2; }
case "$apps" in minimal|full) ;; *) echo "--apps is minimal or full" >&2; exit 2 ;; esac
[ "$(id -u)" = 0 ] || { echo "run as root (sudo)" >&2; exit 1; }
[ -b "$disk" ] || { echo "$disk is not a block device" >&2; exit 1; }

echo "This erases everything on $disk and installs SlateOS for user '$user' (host '$host')."
if [ "$yes" != 1 ]; then
  read -r -p "Type the disk name again to continue: " confirm
  [ "$confirm" = "$disk" ] || { echo "aborted"; exit 1; }
fi

if [ -z "$hash" ]; then
  echo "== password for $user"
  while [ -z "$hash" ]; do
    hash=$(mkpasswd -m sha-512 2>/dev/null || true)
  done
fi

passphrase=""
if [ "$encrypt" = 1 ]; then
  if [ -n "$passfile" ]; then
    passphrase=$(cat "$passfile")
  else
    echo "== disk passphrase (asked at every boot; without it the data cannot be read)"
    while :; do
      read -r -s -p "Passphrase: " p1; echo
      read -r -s -p "Again: " p2; echo
      if [ -z "$p1" ]; then echo "The passphrase is empty."
      elif [ "$p1" != "$p2" ]; then echo "The two do not match."
      else passphrase="$p1"; break
      fi
    done
  fi
  [ -n "$passphrase" ] || { echo "empty disk passphrase" >&2; exit 1; }
fi

part() { if [[ "$disk" == *[0-9] ]]; then echo "${disk}p$1"; else echo "${disk}$1"; fi; }
echo "== partitioning $disk"
parted -s "$disk" mklabel gpt
parted -s "$disk" mkpart ESP fat32 1MiB 513MiB
parted -s "$disk" set 1 esp on
parted -s "$disk" mkpart slateos btrfs 513MiB 100%
udevadm settle
mkfs.fat -F 32 -n BOOT "$(part 1)"
root="$(part 2)"
if [ "$encrypt" = 1 ]; then
  echo "== encrypting $(part 2)"
  printf '%s' "$passphrase" | cryptsetup luksFormat --type luks2 --batch-mode --key-file - "$(part 2)"
  printf '%s' "$passphrase" | cryptsetup open --key-file - "$(part 2)" slateos
  root=/dev/mapper/slateos
fi
mkfs.btrfs -f -L slateos "$root"

echo "== subvolumes"
mkdir -p /mnt
mount "$root" /mnt
btrfs subvolume create /mnt/@
btrfs subvolume create /mnt/@nix
btrfs subvolume create /mnt/@home
btrfs subvolume create "/mnt/@home/$user"
umount /mnt
opts="compress=zstd,noatime"
mount -o "subvol=@,$opts" "$root" /mnt
mkdir -p /mnt/nix /mnt/home /mnt/boot
mount -o "subvol=@nix,$opts" "$root" /mnt/nix
mount -o "subvol=@home,$opts" "$root" /mnt/home
mount "$(part 1)" /mnt/boot

echo "== configuration"
# The live image provides both; SLATEOS_SRC and SLATEOS_TEMPLATE point elsewhere (tests).
src=${SLATEOS_SRC:-/etc/slateos/src}
template=${SLATEOS_TEMPLATE:-/etc/slateos/template.nix}
mkdir -p /mnt/etc/slateos
# nixos-generate-config sees the LUKS container under the root and adds it to the
# initrd (boot.initrd.luks.devices) in hardware-configuration.nix.
nixos-generate-config --root /mnt --dir /mnt/etc/slateos >/dev/null
cp -r "$src" /mnt/etc/slateos/slate
chmod -R u+w /mnt/etc/slateos/slate
luks=""
if [ "$encrypt" = 1 ]; then
  # SSDs keep their speed: TRIM passes through (it reveals which blocks are free).
  luks='boot.initrd.luks.devices."slateos".allowDiscards = true;'
fi
sed -e "s|@USER@|$user|g" -e "s|@HOST@|$host|g" -e "s|@TZ@|$tz|g" -e "s|@HASH@|$hash|g" \
  -e "s|@APPS@|$apps|g" -e "s|@LUKS@|$luks|g" \
  "$template" > /mnt/etc/slateos/configuration.nix
ln -sfn /etc/slateos /mnt/etc/nixos

if [ "$dry" = 1 ]; then
  echo "== dry run: stopping before nixos-install; the target is mounted under /mnt"
  findmnt -R /mnt | head -8
  btrfs subvolume list /mnt
  sed -n '1,30p' /mnt/etc/slateos/configuration.nix
  exit 0
fi

echo "== installing (this copies the system from the live image; no network needed)"
nixos-install --root /mnt --no-root-passwd -I "nixos-config=/mnt/etc/slateos/configuration.nix"
chown -R "$(stat -c %u /mnt/home/"$user" 2>/dev/null || echo 1000):100" "/mnt/home/$user" || true

echo
echo "SlateOS is installed. Remove the medium and reboot; the desktop logs in as $user."
echo "Sign in to your agent from Settings → AI, then press Super+s."
