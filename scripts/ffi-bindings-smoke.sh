#!/bin/sh
# FFI bindings smoke (P7 tasks 9.1/9.2/9.7): regenerate the versioned Swift
# and Kotlin bindings from the built library, then compile and run a real
# host in each language against it. Proves the bindings are current, the
# async handle surface works, and the v1 read-only surface still answers.
set -eu
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

echo "== build the ffi library and bindgen =="
cargo build -p diskgraph-ffi --locked >/dev/null

echo "== regenerate versioned bindings into dist/ffi =="
rm -rf dist/ffi
target/debug/uniffi-bindgen generate \
  --library target/debug/libdiskgraph_ffi.dylib \
  --language swift --out-dir dist/ffi/swift >/dev/null
target/debug/uniffi-bindgen generate \
  --library target/debug/libdiskgraph_ffi.dylib \
  --language kotlin --out-dir dist/ffi/kotlin >/dev/null 2>&1

WORK="$(mktemp -d /tmp/diskgraph-ffi-smoke.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/host" "$WORK/data"
echo "hello diskgraph" > "$WORK/host/README.md"
dd if=/dev/zero of="$WORK/host/blob.bin" bs=1024 count=64 2>/dev/null

echo "== sqlite symbol coexistence (9.3 evidence) =="
SYMBOLS=$(nm -gU target/debug/libdiskgraph_ffi.a 2>/dev/null | grep -c " T _*sqlite3_" || true)
echo "static library exports $SYMBOLS sqlite3_ symbols; the host links the system libsqlite3 in the same process"

echo "== Swift host =="
# Top-level statements are only allowed in a file literally named main.swift.
cp scripts/ffi-smoke-host.swift "$WORK/main.swift"
swiftc "$WORK/main.swift" dist/ffi/swift/diskgraph_ffi.swift \
  -I dist/ffi/swift \
  -Xcc -fmodule-map-file=dist/ffi/swift/diskgraph_ffiFFI.modulemap \
  -L target/debug -ldiskgraph_ffi -lsqlite3 \
  -o "$WORK/swift-host"
DYLD_LIBRARY_PATH="$PWD/target/debug" "$WORK/swift-host" "$WORK/host" "$WORK/data/swift.sqlite"

echo "== Kotlin host =="
JNA="$(find "$HOME/.m2/repository/net/java/dev/jna/jna" -name 'jna-5*.jar' | sort -V | tail -1)"
if [ -z "$JNA" ]; then
  echo "no jna jar in ~/.m2; install one or skip the Kotlin run" >&2
  exit 1
fi
# The main class name follows the source file name; pin it.
cp scripts/ffi-smoke-host.kt "$WORK/KotlinHost.kt"
kotlinc -cp "$JNA" dist/ffi/kotlin/uniffi/diskgraph_ffi/diskgraph_ffi.kt \
  "$WORK/KotlinHost.kt" -include-runtime -d "$WORK/kotlin-host.jar" 2>&1 \
  | grep -v "^warning" || true
java -cp "$WORK/kotlin-host.jar:$JNA" \
  -Djna.library.path="$PWD/target/debug" KotlinHostKt "$WORK/host" "$WORK/data/kotlin.sqlite"

echo "== all FFI binding smoke checks passed =="
