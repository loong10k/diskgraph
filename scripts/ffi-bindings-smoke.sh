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
if [ "${1:-}" = "--swift-only" ]; then
  echo "Swift FFI host checks passed; Kotlin was not requested by this job"
  exit 0
fi
JNA="${JNA_JAR:-$WORK/jna-5.17.0.jar}"
if [ ! -f "$JNA" ]; then
  curl --fail --location --silent --show-error https://repo.maven.apache.org/maven2/net/java/dev/jna/jna/5.17.0/jna-5.17.0.jar -o "$JNA"
fi
JNA_SHA="$(shasum -a 256 "$JNA" | cut -d ' ' -f1)"
if [ "$JNA_SHA" != "b3a9408e7c51e08ef0e3bfcc08f443f6ec0f6191ba8cd7c18d53d2b22e5bdbc0" ]; then
  echo "JNA 5.17.0 digest mismatch" >&2
  exit 1
fi
# The main class name follows the source file name; pin it.
cp scripts/ffi-smoke-host.kt "$WORK/KotlinHost.kt"
if ! kotlinc -cp "$JNA" dist/ffi/kotlin/uniffi/diskgraph_ffi/diskgraph_ffi.kt \
  "$WORK/KotlinHost.kt" -include-runtime -d "$WORK/kotlin-host.jar" >"$WORK/kotlin-build.log" 2>&1; then
  cat "$WORK/kotlin-build.log" >&2
  exit 1
fi
java -cp "$WORK/kotlin-host.jar:$JNA" \
  -Djna.library.path="$PWD/target/debug" KotlinHostKt "$WORK/host" "$WORK/data/kotlin.sqlite"

echo "== all FFI binding smoke checks passed =="
