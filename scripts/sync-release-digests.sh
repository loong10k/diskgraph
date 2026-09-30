#!/bin/sh
# Fills the per-architecture SHA-256 digests in every downstream manifest
# from a published GitHub release. Run after tagging and the release
# workflow completing (see RELEASING.md).
#
#   scripts/sync-release-digests.sh v0.1.0
#
# Updates in place:
#   ../homebrew-diskgraph/Formula/diskgraph.rb   (both architectures)
#   packaging/winget/loong10k.DiskGraph.yaml      (windows)
#   packaging/scoop/diskgraph.json                (windows, both pins)
#
# The release's own SHA256SUMS is the source of truth: nothing is
# recomputed here, so a manifest can never disagree with the release.
set -eu

TAG="${1:?usage: sync-release-digests.sh v0.1.0}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="loong10k/diskgraph"
TAP_DIR="${DISKGRAPH_TAP_DIR:-$ROOT/../homebrew-diskgraph}"

sum_for() {
  target="$1"
  sums=$(mktemp)
  if [ -n "${DISKGRAPH_RELEASE_BASE:-}" ]; then
    curl -fsSL "${DISKGRAPH_RELEASE_BASE%/}/$TAG/SHA256SUMS" -o "$sums"
  else
    curl -fsSL "https://github.com/$REPO/releases/download/$TAG/SHA256SUMS" -o "$sums"
  fi
  digest=$(awk -v name="diskgraph-$target" 'index($2, name) == 1 { print $1 }' "$sums")
  rm -f "$sums"
  if [ -z "$digest" ]; then
    echo "no digest for $target in SHA256SUMS" >&2
    exit 1
  fi
  printf '%s' "$digest"
}

ARM_MAC="$(sum_for aarch64-apple-darwin)"
X86_MAC="$(sum_for x86_64-apple-darwin)"
WIN="$(sum_for x86_64-pc-windows-msvc)"

echo "digests for $TAG:"
echo "  aarch64-apple-darwin   $ARM_MAC"
echo "  x86_64-apple-darwin    $X86_MAC"
echo "  x86_64-pc-windows-msvc $WIN"

if [ -f "$TAP_DIR/Formula/diskgraph.rb" ]; then
  formula="$TAP_DIR/Formula/diskgraph.rb"
  python3 - "$formula" "$ARM_MAC" "$X86_MAC" <<'PY'
import re, sys
path, arm, intel = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(path).read()
text = text.replace("REPLACE_ARM64_SHA256", arm).replace("REPLACE_X86_64_SHA256", intel)
open(path, "w").write(text)
print(f"updated {path}")
PY
else
  echo "tap formula not found at $TAP_DIR (skipped; clone the tap or set DISKGRAPH_TAP_DIR)" >&2
fi

python3 - "$ROOT" "$WIN" <<'PY'
import json, sys, re
root, win = sys.argv[1], sys.argv[2]
winget = f"{root}/packaging/winget/loong10k.DiskGraph.yaml"
text = open(winget).read().replace("REPLACE_WINDOWS_SHA256", win)
open(winget, "w").write(text)
print(f"updated {winget}")
scoop = f"{root}/packaging/scoop/diskgraph.json"
data = json.load(open(scoop))
data["hash"] = win
data["autoupdate"]["hash"] = win
json.dump(data, open(scoop, "w"), indent=2)
open(scoop, "a").write("\n")
print(f"updated {scoop}")
PY

echo "done; review the diffs, then push the tap and open the manifest PRs"
