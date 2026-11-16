#!/usr/bin/env bash
# check-image.sh: the image must hold none of the agent programs.
#
#   distro/release/check-image.sh [SYSTEM]     after `nix build .#iso` (default: this machine's)
#
# Claude Code is unfree, and SlateOS has no right to hand it on: the image names the
# agent programs (/etc/slateos/agents.json: which version, where from) and every
# computer downloads them itself. This lists what the image's store holds, the closure
# of its system, and fails when one of the programs named there is in it.
set -euo pipefail

system=${1:-$(nix eval --raw --impure --expr builtins.currentSystem)}
config=".#nixosConfigurations.iso-$system.config"
top=$(nix eval --raw "$config.system.build.toplevel.outPath")
nix-store --check-validity "$top" 2>/dev/null || { echo "$top is not built: nix build .#iso first" >&2; exit 1; }
held=$(nix-store -qR "$top")
bad=0
while read -r name out; do
  if grep -qxF -- "$out" <<< "$held"; then
    echo "::error::the image holds $name ($out)"
    bad=1
  fi
done < <(nix eval --raw "$config.environment.etc.\"slateos/agents.json\".text" | jq -r 'to_entries[] | "\(.key) \(.value.out)"')
if [ "$bad" = 0 ]; then
  echo "The image holds none of the agent programs it names ($(wc -l <<< "$held") store paths looked at)."
fi
exit "$bad"
