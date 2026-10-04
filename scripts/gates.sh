#!/bin/sh
# The repository's quality gates. Format and lint run crate by crate on
# purpose: `cargo fmt --all` follows path dependencies and would rewrite the
# vendored upstream scanner, whose bytes are verified against a digest pin
# (crates/diskgraph-disktree-core/VENDORED.md).
set -eu
cd "$(dirname "$0")/.."

CRATES="diskgraph-core diskgraph-store diskgraph-disktree diskgraph-scan-worker diskgraph-ffi \
diskgraph-testkit diskgraph-engine diskgraph-cli diskgraph-mcp diskgraph-ops"

fmt_args=""
for crate in $CRATES; do
  fmt_args="$fmt_args -p $crate"
done

echo "==> cargo fmt (workspace crates; vendored scanner excluded)"
# shellcheck disable=SC2086
cargo fmt $fmt_args --check

echo "==> cargo clippy"
# shellcheck disable=SC2086
cargo clippy $fmt_args --all-targets --locked -- -D warnings

echo "==> cargo test"
# shellcheck disable=SC2086
cargo test $fmt_args --locked -- --test-threads=1

echo "==> vendored scanner builds and passes its own tests"
(cd crates/diskgraph-disktree-core && cargo test --quiet)

echo "all gates green"
