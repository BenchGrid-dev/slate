# slateos-install: put SlateOS on a disk. Run from the live image as root.
#
#   sudo slateos-install --disk /dev/nvme0n1 --user alice --host mybox [--tz Europe/Berlin]
#   options for scripted use: --yes, --password-hash HASH, --dry-run
#
# Layout: GPT, 512 MiB EFI system partition, the rest btrfs with subvolumes
# @ (root), @nix, @home, and @home/<user> owned by the user, which is what Slate's
# snapshots and undo need. Everything on the disk is erased.
set -euo pipefail

disk="" user="" host="slateos" tz="UTC" dry=0 yes=0 hash=""
while [ $# -gt 0 ]; do
  case "$1" in
    --disk) disk="$2"; shift 2 ;;
    --user) user="$2"; shift 2 ;;
    --host) host="$2"; shift 2 ;;
    --tz) tz="$2"; shift 2 ;;
    --password-hash) hash="$2"; shift 2 ;;   # for scripted installs; otherwise asked
    --yes) yes=1; shift ;;                   # skip the disk confirmation (scripted installs)
    --dry-run) dry=1; shift ;;               # partition, subvolumes and configuration only
    -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$disk" ] && [ -n "$user" ] || { echo "usage: slateos-install --disk DEV --user NAME [--host NAME] [--tz ZONE]" >&2; exit 2; }
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

part() { if [[ "$disk" == *[0-9] ]]; then echo "${disk}p$1"; else echo "${disk}$1"; fi; }
echo "== partitioning $disk"
parted -s "$disk" mklabel gpt
parted -s "$disk" mkpart ESP fat32 1MiB 513MiB
parted -s "$disk" set 1 esp on
parted -s "$disk" mkpart slateos btrfs 513MiB 100%
udevadm settle
mkfs.fat -F 32 -n BOOT "$(part 1)"
mkfs.btrfs -f -L slateos "$(part 2)"

echo "== subvolumes"
mkdir -p /mnt
mount "$(part 2)" /mnt
btrfs subvolume create /mnt/@
btrfs subvolume create /mnt/@nix
btrfs subvolume create /mnt/@home
btrfs subvolume create "/mnt/@home/$user"
umount /mnt
opts="compress=zstd,noatime"
mount -o "subvol=@,$opts" "$(part 2)" /mnt
mkdir -p /mnt/nix /mnt/home /mnt/boot
mount -o "subvol=@nix,$opts" "$(part 2)" /mnt/nix
mount -o "subvol=@home,$opts" "$(part 2)" /mnt/home
mount "$(part 1)" /mnt/boot

echo "== configuration"
mkdir -p /mnt/etc/slateos
nixos-generate-config --root /mnt --dir /mnt/etc/slateos >/dev/null
cp -r /etc/slateos/src /mnt/etc/slateos/slate
chmod -R u+w /mnt/etc/slateos/slate
sed -e "s|@USER@|$user|g" -e "s|@HOST@|$host|g" -e "s|@TZ@|$tz|g" -e "s|@HASH@|$hash|g" \
  /etc/slateos/template.nix > /mnt/etc/slateos/configuration.nix
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
