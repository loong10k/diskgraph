#!/bin/sh
set -eu
mkdir -p /tmp/diskgraph-source /tmp/diskgraph-cargo
cd /tmp/diskgraph-source
tar -xf /evidence/source.tar
rustc -Vv > /evidence/environment.txt
cargo -V >> /evidence/environment.txt
uname -a >> /evidence/environment.txt
id >> /evidence/environment.txt
store_status=0
cargo test -p diskgraph-store --lib --locked -- --test-threads=1 > /evidence/store.log 2>&1 || store_status=$?
benchmark_status=0
cargo test --release -p diskgraph-store --lib --locked policy_store::control_query_cache_tests::measure_control_query_compilation_cost -- --ignored --exact --nocapture > /evidence/release.log 2>&1 || benchmark_status=$?
printf 'store=%s\nbenchmark=%s\n' "$store_status" "$benchmark_status" > /evidence/status.txt
if [ "$store_status" -ne 0 ] || [ "$benchmark_status" -ne 0 ]; then exit 1; fi
