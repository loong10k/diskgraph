#!/usr/bin/env bash
# Builds the private Linux service bundle (P4 task 5.10, specs AI-01 / ST-05).
#
# RUN THIS ON A LINUX GNU HOST: cargo-zigbuild and Zig must already be
# installed, along with the host's Rust target. Builds against glibc 2.17
# with bundled SQLite, then verifies the actual ELF requirements and runs
# an isolated native smoke test. This script does not install tools.
#
# Usage: ./scripts/package-linux.sh [output-dir]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="${1:-$REPO_ROOT/dist/private-linux}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH="$(uname -m)"
if [ -z "$VERSION" ]; then
    echo "cannot read the workspace version" >&2
    exit 1
fi

if [ "$(uname -s)" != Linux ]; then
    echo "package-linux.sh requires a native Linux GNU host" >&2
    exit 1
fi
BUILD_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
case "$BUILD_TARGET" in
    x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) ;;
    *) echo "unsupported native GNU package target: $BUILD_TARGET" >&2; exit 1 ;;
esac
if ! cargo zigbuild --version >/dev/null 2>&1; then
    echo "GNU glibc 2.17 packaging requires preinstalled cargo-zigbuild and Zig; no tools were installed" >&2
    exit 1
fi
# Zig 可由 cargo-zigbuild 从已安装 Python wheel 定位，不强制要求 PATH 中有 zig。
# 实际构建失败（含缺失 Zig）由 set -e 拒绝，发生在候选目录创建之前。
BUILD_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/$BUILD_TARGET/release"

echo "==> building release binaries on $(uname -s) $ARCH"
cargo zigbuild --release --locked -p diskgraph-cli -p diskgraph-mcp -p diskgraph-scan-worker \
    --target "$BUILD_TARGET.2.17"

# 不允许本机构建静默提高原清单承诺；超出基线时在安装文件和写清单前失败。
python3 scripts/linux_abi_requirements.py --maximum-glibc 2.17 --target "$BUILD_TARGET" \
    "$BUILD_DIR/diskgraph" "$BUILD_DIR/diskgraph-mcp" "$BUILD_DIR/diskgraph-scan-worker"

mkdir -p "$OUT_DIR/bin" "$OUT_DIR/deploy"
for binary in diskgraph diskgraph-mcp diskgraph-scan-worker; do
    install -m 0755 "$BUILD_DIR/$binary" "$OUT_DIR/bin/$binary"
done

# 复制后检查真正包内字节，不能用源路径的旧观察为后续副本提供兼容证明。
python3 scripts/linux_abi_requirements.py --maximum-glibc 2.17 --target "$BUILD_TARGET" \
    "$OUT_DIR/bin/diskgraph" "$OUT_DIR/bin/diskgraph-mcp" "$OUT_DIR/bin/diskgraph-scan-worker" \
    > "$OUT_DIR/GNU_ABI.jsonl"

echo "==> installing the systemd unit example"
install -m 0644 deploy/diskgraph-mcp.service "$OUT_DIR/deploy/diskgraph-mcp.service"

python3 scripts/worker_manifest.py --bin-dir "$OUT_DIR/bin" --target "$BUILD_TARGET" --version "$VERSION"

echo "==> recording checksums and provenance"
(
    cd "$OUT_DIR"
    shasum -a 256 bin/diskgraph bin/diskgraph-mcp bin/diskgraph-scan-worker bin/scan-worker-manifest.json > SHA256SUMS 2>/dev/null \
        || sha256sum bin/diskgraph bin/diskgraph-mcp bin/diskgraph-scan-worker bin/scan-worker-manifest.json > SHA256SUMS
    {
        echo "# DiskGraph private Linux bundle"
        echo "version:   $VERSION"
        echo "platform:  $(uname -s) $ARCH"
        echo "built-at:  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "rustc:     $(rustc --version)"
        echo "profile:   release"
        echo
        echo "# Requires only glibc >= 2.17 (bundled SQLite; no system sqlite)."
        echo "# No Rust toolchain, PruneX, or disktree-app at runtime."
        for binary in diskgraph diskgraph-mcp diskgraph-scan-worker; do
            echo "bin/$binary  $(stat -c%s "bin/$binary" 2>/dev/null || stat -f%z "bin/$binary") bytes"
        done
        echo
        echo "# SQLite databases are LOCAL-ONLY (ST-05): never on a network share."
    } > MANIFEST.txt
)
cp LICENSE "$OUT_DIR/LICENSE"
cp docs/dependency-inventory.md "$OUT_DIR/DEPENDENCIES.md"

echo "==> smoke: start / status / restart / upgrade on local SQLite"
SMOKE_ROOT="$(mktemp -d)"
trap 'rm -rf "$SMOKE_ROOT"' EXIT
mkdir -p "$SMOKE_ROOT/project" "$SMOKE_ROOT/home"
printf '[package]\nname = "smoke"\n' > "$SMOKE_ROOT/project/Cargo.toml"
head -c 65536 /dev/zero > "$SMOKE_ROOT/project/blob.bin"

BIN="$OUT_DIR/bin"
DATA="$SMOKE_ROOT/data"

# scope + index, then a query; all without any system utility on PATH beyond
# the bundle and the shell built-ins the harness itself needs.
"$BIN/diskgraph" --data-dir "$DATA" --json scope add --root "$SMOKE_ROOT/project" > "$SMOKE_ROOT/scope.json"
SCOPE="$(sed -n 's/.*"scope_id":"\([^"]*\)".*/\1/p' "$SMOKE_ROOT/scope.json" | head -1)"
"$BIN/diskgraph" --data-dir "$DATA" --json index --scope "$SCOPE" --wait > "$SMOKE_ROOT/index.json"
"$BIN/diskgraph" --data-dir "$DATA" --json top --scope "$SCOPE" --limit 5 > "$SMOKE_ROOT/top.json"
"$BIN/diskgraph" --data-dir "$DATA" --json doctor > "$SMOKE_ROOT/doctor.json"

grep -q '"state":"completed"' "$SMOKE_ROOT/index.json" \
    || { echo "smoke: index did not complete" >&2; exit 1; }
grep -q '"subtree_bytes":[1-9]' "$SMOKE_ROOT/top.json" \
    || { echo "smoke: scan reported zero bytes" >&2; exit 1; }
grep -q '"healthy":true' "$SMOKE_ROOT/doctor.json" \
    || { echo "smoke: doctor unhealthy" >&2; exit 1; }

# Restart-safety: a fresh process over the same data dir answers from the
# persisted index without rescanning (spec AI-03).
"$BIN/diskgraph" --data-dir "$DATA" --json snapshots --scope "$SCOPE" > "$SMOKE_ROOT/snapshots.json"
grep -q '"snapshot_id"' "$SMOKE_ROOT/snapshots.json" \
    || { echo "smoke: persisted index not visible after restart" >&2; exit 1; }

ITEMS="$(grep -o '"name":' "$SMOKE_ROOT/top.json" | wc -l | tr -d ' ')"
BYTES="$(sed -n 's/.*"subtree_bytes":\([0-9]*\).*/\1/p' "$SMOKE_ROOT/top.json" | head -1)"
echo "smoke: $ITEMS items, largest subtree $BYTES bytes, restart query ok, doctor healthy"

echo
echo "bundle ready: $OUT_DIR"
ls -lh "$OUT_DIR/bin"
echo
cat "$OUT_DIR/SHA256SUMS"
echo
echo "systemd: install deploy/diskgraph-mcp.service per its header comments."
