# slateos-live-agents: bring agent programs into the live session. Run as root.
#
#   sudo slateos-live-agents [NAME...]    claude-code, codex (default: every one)
#   slateos-live-agents --status          which of them the live session has
#
# SlateOS's image holds no agent program: an installation downloads them to the disk.
# The live session needs one earlier when a person signs in with a subscription, or
# wants to try the agent before installing. This downloads it into the live session's
# store, which is memory, and puts it on everyone's PATH. An installation afterwards
# copies it to the disk instead of downloading it again.
set -euo pipefail

list=${SLATEOS_AGENTS:-/etc/slateos/agents.json}
case "${1:-}" in -h|--help) sed -n '/^# slateos-live-agents:/,/^[^#]/{/^#/p;}' "$0"; exit 0 ;; esac
[ -r "$list" ] || { echo "$list is missing: this is not the SlateOS live image" >&2; exit 1; }
field() { jq -r --arg n "$1" --arg f "$2" '.[$n][$f] // empty' "$list"; }
if [ "${1:-}" = --status ]; then
  while read -r name; do
    if nix-store --check-validity "$(field "$name" out)" 2>/dev/null; then state=here; else state=missing; fi
    echo "$name $(field "$name" version) $state"
  done < <(jq -r 'keys[]' "$list")
  exit 0
fi
[ "$(id -u)" = 0 ] || { echo "run as root (sudo)" >&2; exit 1; }
if [ $# -gt 0 ]; then names=("$@"); else mapfile -t names < <(jq -r 'keys[]' "$list"); fi

# What one program takes while it is put together, in MiB: the download and the copy of
# it that stays. The store is memory here, and so is everything else.
need=800
room() {
  local store memory
  store=$(df -Pm /nix/store | awk 'NR == 2 { print $4 }')
  memory=$(awk '/^MemAvailable:/ { print int($2 / 1024) }' /proc/meminfo)
  echo $(( store < memory ? store : memory ))
}

got=()
for name in "${names[@]}"; do
  out=$(field "$name" out)
  [ -n "$out" ] || { echo "unknown agent program $name (the image knows: $(jq -r 'keys | join(", ")' "$list"))" >&2; exit 2; }
  if nix-store --check-validity "$out" 2>/dev/null; then
    echo "== $name is here already"
    got+=("$out")
    continue
  fi
  echo "== downloading $name $(field "$name" version)"
  check=$(field "$name" check)
  host=${check#*://}
  host=${host%%/*}
  if ! curl -fsS -o /dev/null --head --max-time 20 "$check"; then
    echo "$host does not answer: $name cannot be downloaded. Check the network connection." >&2
    exit 1
  fi
  free=$(room)
  if [ "$free" -lt "$need" ]; then
    echo "The live session has too little memory left for $name (${free} MB free, it takes about ${need})." \
      "Install SlateOS without it: the installation downloads it to the disk." >&2
    exit 3
  fi
  drv=$(field "$name" drv)
  if [ -n "$drv" ]; then
    # Not in a binary cache: the file comes from its maker and is put together here,
    # with tools the image holds.
    nix-store --realise "$drv" >/dev/null
    # The downloaded file is in the program now; its own copy only takes memory.
    src=$(field "$name" src)
    if [ -n "$src" ]; then nix-store --delete "$src" >/dev/null 2>&1 || true; fi
  else
    nix-store --realise "$out" >/dev/null
  fi
  nix-store --check-validity "$out"
  got+=("$out")
done

# On everyone's PATH, the running session's too.
nix-env --profile /nix/var/nix/profiles/default --install "${got[@]}" >/dev/null
echo "== done"
