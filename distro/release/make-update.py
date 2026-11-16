#!/usr/bin/env python3
"""make-update: what a release publishes for software update.

    distro/release/make-update.py VERSION --key FILE [--out DIR] [--no-cache]
                                  [--from VERSION] [--step URL]

Writes into DIR (default: ./release-VERSION):

    slateos-VERSION-source.tar.gz   the tree at tag vVERSION, under slateos-VERSION/
    slateos-update.json             the manifest: version, date, summary, the source's checksum
    slateos-update.json.sig         its Ed25519 signature, by the key in FILE

All three are attached to the GitHub release; installed systems read
releases/latest/download/slateos-update.json. The Release workflow runs this for a tag
(docs/releasing.md). FILE is the release key (`slateos-update keygen`): whoever has it
can sign an update every SlateOS installation accepts.

--from names the oldest version that can take this release directly, and --step the
manifest older systems are sent to first (the release in between). The manifest names
the binary cache the Release workflow filled for this version (distro/release/cache.json;
--cache-url and --cache-key for another, --no-cache for none).
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def git(*args):
    return subprocess.run(["git", "-C", REPO, *args], check=True, capture_output=True, text=True).stdout.strip()


def changelog(version):
    """The date and the opening paragraph of a version's section in CHANGELOG.md."""
    with open(os.path.join(REPO, "CHANGELOG.md"), encoding="utf-8") as f:
        text = f.read()
    m = re.search(rf"^## {re.escape(version)} — (\d{{4}}-\d{{2}}-\d{{2}})\n\n(.*?)\n\n", text, re.M | re.S)
    if not m:
        sys.exit(f"CHANGELOG.md has no section \"## {version} — DATE\" with an opening paragraph")
    return m.group(1), " ".join(m.group(2).split())


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("version")
    ap.add_argument("--key", required=True, help="the release key file")
    ap.add_argument("--out", help="output directory (default ./release-VERSION)")
    ap.add_argument("--from", dest="from_version", help="oldest version that can update to this one directly")
    ap.add_argument("--step", help="manifest address for systems older than --from")
    ap.add_argument("--cache-url", help="the binary cache with this release's prebuilt packages (default: distro/release/cache.json)")
    ap.add_argument("--cache-key", help="the public key that cache's store paths are signed with (name:base64)")
    ap.add_argument("--no-cache", action="store_true", help="name no binary cache: updates build Slate on the person's computer")
    ap.add_argument("--signer", help="the slateos-update program (default: cargo run -p slate-update)")
    args = ap.parse_args()

    version, tag = args.version, f"v{args.version}"
    try:
        git("rev-parse", "-q", "--verify", f"refs/tags/{tag}")
    except subprocess.CalledProcessError:
        sys.exit(f"there is no tag {tag}; tag the release first")
    in_tree = re.search(r'^version = "(.*)"$', git("show", f"{tag}:Cargo.toml"), re.M).group(1)
    if in_tree != version:
        sys.exit(f"tag {tag} holds version {in_tree}")
    released, summary = changelog(version)

    out = args.out or os.path.join(os.getcwd(), f"release-{version}")
    os.makedirs(out, exist_ok=True)
    name = f"slateos-{version}-source.tar.gz"
    archive = os.path.join(out, name)
    subprocess.run(["git", "-C", REPO, "archive", "--format=tar.gz", f"--prefix=slateos-{version}/", "-o", archive, tag], check=True)
    with open(archive, "rb") as f:
        data = f.read()

    manifest = {
        "format": 1,
        "version": version,
        "released": released,
        "summary": summary,
        "notes": f"https://github.com/BenchGrid-dev/slate/releases/tag/{tag}",
        "source": {"url": name, "sha256": hashlib.sha256(data).hexdigest(), "size": len(data)},
    }
    if args.from_version:
        manifest["from"] = args.from_version
    if args.step:
        manifest["step"] = args.step
    if bool(args.cache_url) != bool(args.cache_key):
        sys.exit("--cache-url and --cache-key go together")
    if args.cache_url:
        manifest["cache"] = {"url": args.cache_url, "key": args.cache_key}
    elif not args.no_cache:
        # Where the Release workflow put this version's packages.
        with open(os.path.join(REPO, "distro", "release", "cache.json"), encoding="utf-8") as f:
            manifest["cache"] = json.load(f)
    path = os.path.join(out, "slateos-update.json")
    with open(path, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, ensure_ascii=False)
        f.write("\n")

    signer = [args.signer] if args.signer else ["cargo", "run", "-q", "--manifest-path", os.path.join(REPO, "Cargo.toml"), "-p", "slate-update", "--"]
    signature = subprocess.run(signer + ["sign", args.key, path], check=True, capture_output=True, text=True).stdout
    with open(path + ".sig", "w", encoding="utf-8") as f:
        f.write(signature)

    print(f"{out}:")
    for file in (name, "slateos-update.json", "slateos-update.json.sig"):
        print(f"  {file}  {os.path.getsize(os.path.join(out, file))} bytes")
    print(f"SlateOS {version}, released {released}: {summary}")


if __name__ == "__main__":
    main()
