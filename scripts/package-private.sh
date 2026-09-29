#!/usr/bin/env bash
# Builds the private macOS release bundle: binaries, checksums, a licence
# manifest, and a smoke test that runs without Rust, PruneX, or disktree-app
# on the machine (P3 task 4.7, specs AI-01 / RE-04 / RE-05).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="${1:-$REPO_ROOT/dist/private-macos}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if [ -z "$VERSION" ]; then
    echo "cannot read the workspace version" >&2
    exit 1
fi

mkdir -p "$OUT_DIR/bin"

echo "==> building release binaries"
cargo build --release --locked -p diskgraph-cli -p diskgraph-mcp

for binary in diskgraph diskgraph-mcp; do
    install -m 0755 "target/release/$binary" "$OUT_DIR/bin/$binary"
done

echo "==> recording checksums and sizes"
(
    cd "$OUT_DIR"
    shasum -a 256 bin/diskgraph bin/diskgraph-mcp > SHA256SUMS
    {
        echo "# DiskGraph private bundle"
        echo "version:   $VERSION"
        echo "platform:  $(uname -s) $(uname -m)"
        echo "built-at:  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "rustc:     $(rustc --version)"
        echo "profile:   release"
        echo
        echo "# No Rust toolchain, PruneX, or disktree-app is required at runtime."
        echo "# Requires only a POSIX system (file/dir metadata via platform APIs)."
        for binary in diskgraph diskgraph-mcp; do
            echo "bin/$binary  $(stat -f%z "bin/$binary") bytes"
        done
    } > MANIFEST.txt
)

echo "==> bundling the dependency inventory"
cp docs/dependency-inventory.md "$OUT_DIR/DEPENDENCIES.md"

echo "==> bundling the licences"
cp LICENSE "$OUT_DIR/LICENSE"
mkdir -p "$OUT_DIR/licenses"
for dependency in disktree-core serde serde_json uuid base64 rusqlite thiserror uniffi clap sha2 hmac hex libc; do
    echo "$dependency" >> "$OUT_DIR/licenses/THIRD-PARTY.txt"
done
{
    echo "# Third-party licences"
    echo
    echo "DiskGraph is MIT (see LICENSE)."
    echo "The following dependencies ship inside the binaries under their own"
    echo "permissive terms; see docs/dependency-inventory.md for versions and"
    echo "full licence expressions."
    echo
    sed 's/^/- /' "$OUT_DIR/licenses/THIRD-PARTY.txt"
} > "$OUT_DIR/licenses/THIRD-PARTY-LICENSES.md"

echo "==> smoke test in an environment without the build toolchain"
# The bundle must answer a full read-only flow with a PATH that contains no
# system utilities and no cargo/rustc.
SMOKE_ROOT="$(mktemp -d)"
trap 'rm -rf "$SMOKE_ROOT"' EXIT
mkdir -p "$SMOKE_ROOT/project/target" "$SMOKE_ROOT/home"
printf '[package]\nname = "smoke"\n' > "$SMOKE_ROOT/project/Cargo.toml"
head -c 65536 /dev/zero > "$SMOKE_ROOT/project/target/app.bin"

CLEAN_PATH="$SMOKE_ROOT/home"
# Absolute paths to the few coreutils the harness itself uses. The PATH handed
# to the binaries stays empty; only the harness reaches for these.
SED_BIN="$(command -v sed)"
GREP_BIN="$(command -v grep)"
WC_BIN="$(command -v wc)"
TR_BIN="$(command -v tr)"
HEAD_BIN="$(command -v head)"
CAT_BIN="$(command -v cat)"
# The bundle must answer a full read-only flow with a PATH that contains no
# system utilities and no toolchain. Parsing stays in the shell for the same
# reason: nothing outside the bundle may be assumed present.
(
    cd "$SMOKE_ROOT"
    export PATH="$CLEAN_PATH"
    BIN="$OUT_DIR/bin"
    DATA="$SMOKE_ROOT/data"

    "$BIN/diskgraph" --data-dir "$DATA" --json scope add --root "$SMOKE_ROOT/project" > scope.json
    # Pull "scope_id":"..." out of the envelope without a JSON parser.
    SCOPE="$("$SED_BIN" -n 's/.*"scope_id":"\([^"]*\)".*/\1/p' scope.json | "$HEAD_BIN" -1)"
    if [ -z "$SCOPE" ]; then
        echo "smoke: scope registration produced no scope id" >&2
        "$CAT_BIN" scope.json >&2
        exit 1
    fi

    "$BIN/diskgraph" --data-dir "$DATA" --json index --scope "$SCOPE" --wait > index.json
    "$BIN/diskgraph" --data-dir "$DATA" --json top --scope "$SCOPE" --limit 5 > top.json
    "$BIN/diskgraph" --data-dir "$DATA" --json doctor > doctor.json

    "$GREP_BIN" -q '"state":"completed"' index.json || { echo "smoke: index did not complete" >&2; "$CAT_BIN" index.json >&2; exit 1; }
    "$GREP_BIN" -q '"ok":true' top.json || { echo "smoke: top failed" >&2; "$CAT_BIN" top.json >&2; exit 1; }
    "$GREP_BIN" -q '"subtree_bytes":[1-9]' top.json || { echo "smoke: scan reported zero bytes" >&2; "$CAT_BIN" top.json >&2; exit 1; }
    "$GREP_BIN" -q '"healthy":true' doctor.json || { echo "smoke: doctor unhealthy" >&2; "$CAT_BIN" doctor.json >&2; exit 1; }

    ITEMS="$("$GREP_BIN" -o '"name":' top.json | "$WC_BIN" -l | "$TR_BIN" -d ' ')"
    BYTES="$("$SED_BIN" -n 's/.*"subtree_bytes":\([0-9]*\).*/\1/p' top.json | "$HEAD_BIN" -1)"
    echo "smoke: $ITEMS items, largest subtree $BYTES bytes, index completed, doctor healthy"
)

echo
echo "bundle ready: $OUT_DIR"
ls -lh "$OUT_DIR/bin"
echo
cat "$OUT_DIR/SHA256SUMS"
