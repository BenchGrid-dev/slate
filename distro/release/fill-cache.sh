#!/usr/bin/env bash
# fill-cache.sh DIR TOPLEVEL...: put into DIR, as a signed Nix binary cache, what the
# closure of these systems holds and cache.nixos.org does not: Slate itself, the
# packages SlateOS patches, and what is generated from them. Updates then copy these
# instead of compiling them (docs/software-update.md).
#
# NIX_CACHE_KEY_FILE names the cache's secret key. Left out on purpose:
#   - unfree packages (Claude Code): their licence is not ours to pass on; a person's
#     computer fetches them from their publisher;
#   - initrds, which are built for one machine's disks and match nobody else's.
#
# Nix only adds a path to a binary cache whose references are in it too, so the whole
# closure is written to a cache beside DIR first, and what belongs in DIR is picked
# from there.
set -euo pipefail
dir=$1; shift
key=${NIX_CACHE_KEY_FILE:?the secret key of the cache}
nix() { command nix --extra-experimental-features "nix-command flakes" "$@"; }

closure=$(nix path-info -r "$@" | sort -u)
echo "closure: $(wc -l <<<"$closure") paths"
upstream() {
  hash=${1#/nix/store/}; hash=${hash%%-*}
  curl -sf --retry 3 -o /dev/null -I "https://cache.nixos.org/$hash.narinfo" || echo "$1"
}
export -f upstream
ours=$(xargs -P 24 -n 1 bash -c 'upstream "$0"' <<<"$closure" | sort -u)
echo "not on cache.nixos.org: $(wc -l <<<"$ours") paths"
keep=$(grep -Ev -- '-claude-code-|-initrd-linux-|-initrd$' <<<"$ours" || true)
echo "for the cache: $(wc -l <<<"$keep") paths"

whole=$(mktemp -d "${TMPDIR:-/tmp}/whole-cache.XXXXXX")
trap 'rm -rf "$whole"' EXIT
nix copy --to "file://$whole?secret-key=$key&compression=zstd" "$@"
mkdir -p "$dir/nar"
cp "$whole/nix-cache-info" "$dir/"
while read -r path; do
  hash=${path#/nix/store/}; hash=${hash%%-*}
  nar=$(sed -n 's/^URL: //p' "$whole/$hash.narinfo")
  cp "$whole/$hash.narinfo" "$dir/"
  cp "$whole/$nar" "$dir/$nar"
done <<<"$keep"
# Nothing that is someone else's to distribute may have slipped in.
if grep -l "^StorePath: .*claude-code" "$dir"/*.narinfo; then
  echo "an unfree package is in the cache" >&2; exit 1
fi
echo "cache: $(find "$dir" -name '*.narinfo' | wc -l) paths, $(du -sh "$dir" | cut -f1)"
sed -n 's|^StorePath: /nix/store/[a-z0-9]*-||p' "$dir"/*.narinfo | sort | tr '\n' ' ' | fold -s -w 160
echo
