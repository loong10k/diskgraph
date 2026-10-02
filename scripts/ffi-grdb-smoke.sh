#!/bin/sh
# PF-02：实际 SwiftPM/GRDB 宿主；只覆盖 macOS 动态链接组合。
set -eu
cd "$(dirname "$0")/.."
PROFILE="${DISKGRAPH_FFI_PROFILE:-release}"
case "$PROFILE" in
  release) cargo build --release -p diskgraph-ffi --locked ;;
  debug) cargo build -p diskgraph-ffi --locked ;;
  *) echo "DISKGRAPH_FFI_PROFILE must be debug or release" >&2; exit 1 ;;
esac
LIBRARY_DIR="$PWD/target/$PROFILE"
# Rust 内部 SQLite 不得通过动态库符号表污染 GRDB 的系统 SQLite。
if nm -gU "$LIBRARY_DIR/libdiskgraph_ffi.dylib" | grep -Eq ' [TDS] _sqlite3_'; then
  echo "Rust dynamic library unexpectedly exports SQLite symbols" >&2
  exit 1
fi
WORK="$(mktemp -d /tmp/diskgraph-grdb.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT
cp -R fixtures/ffi_grdb_host "$WORK/package"
mkdir -p "$WORK/package/bindings/ffi" "$WORK/package/bindings/swift" "$WORK/root" "$WORK/data"
"$LIBRARY_DIR/uniffi-bindgen" generate --library "$LIBRARY_DIR/libdiskgraph_ffi.dylib" \
  --language swift --no-format --out-dir "$WORK/generated"
cp "$WORK/generated/diskgraph_ffi.swift" "$WORK/package/bindings/swift/"
cp "$WORK/generated/diskgraph_ffiFFI.h" "$WORK/package/bindings/ffi/"
cp "$WORK/generated/diskgraph_ffiFFI.modulemap" "$WORK/package/bindings/ffi/module.modulemap"
python3 - "$WORK/root" <<'PY'
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
for index in range(2000):
    (root / f"file-{index:04d}.txt").write_text(f"fixture-{index}\n", encoding="utf-8")
PY
DISKGRAPH_RUST_LIBRARY_DIR="$LIBRARY_DIR" DYLD_LIBRARY_PATH="$LIBRARY_DIR" \
  swift run --package-path "$WORK/package" --scratch-path "$PWD/target/ffi-grdb-swift" \
  --force-resolved-versions --configuration release FfiGrdbHost "$WORK/root" "$WORK/data"
