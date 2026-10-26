#!/bin/sh
# 仅在临时 Docker 容器内预置 root 夹具；不安装宿主服务。
set -eu
docker run --rm --network none --memory 4g --cpus 4 -v diskgraph-production-linux-arm64-d101:/build -v /Users/wandl/workspaces/workspace-loong10k/diskgraph:/src:ro -e CARGO_HOME=/build/cargo -e CARGO_TARGET_DIR=/build/terminal-red-target -e CARGO_PROFILE_TEST_DEBUG=0 -e CARGO_PROFILE_DEV_DEBUG=0 -e CARGO_INCREMENTAL=0 rust@sha256:874db0adb75ee22be0fa0ba19dec70270456cd3db0f9385c06eaea33f09d4248 sh -c 'set -eu
cp /src/crates/diskgraph-engine/src/recovery_slot/linux_supervisor_namespace_tests.rs /build/terminal-red-7fc3330/crates/diskgraph-engine/src/recovery_slot/linux_supervisor_namespace_tests.rs
cp /src/crates/diskgraph-engine/src/recovery_slot/linux_supervisor_namespace.rs /src/crates/diskgraph-engine/src/recovery_slot/linux_supervisor_trust.rs /src/crates/diskgraph-engine/src/recovery_slot/linux_supervisor_peer.rs /src/crates/diskgraph-engine/src/recovery_slot/linux_supervisor_materials.rs /src/crates/diskgraph-engine/src/recovery_slot/linux_supervisor_materials_tests.rs /build/terminal-red-7fc3330/crates/diskgraph-engine/src/recovery_slot/
cp /src/crates/diskgraph-engine/src/recovery_slot/mod.rs /build/terminal-red-7fc3330/crates/diskgraph-engine/src/recovery_slot/mod.rs
chown 1000:1000 /build/terminal-red-7fc3330/crates/diskgraph-engine/src/recovery_slot/mod.rs
mkdir -p /fixture/supervisor_namespace
chmod 755 /fixture /fixture/supervisor_namespace
for case in good parallel writable user_owned linked_slot symlink_slot wrong_owner writable_slot missing_slot; do
mkdir /fixture/supervisor_namespace/$case
chmod 755 /fixture/supervisor_namespace/$case
done
for case in good parallel linked_slot wrong_owner writable_slot; do
: > /fixture/supervisor_namespace/$case/uid_1000.slot
chmod 600 /fixture/supervisor_namespace/$case/uid_1000.slot
done
chown 1000:1000 /fixture/supervisor_namespace/parallel/uid_1000.slot /fixture/supervisor_namespace/good/uid_1000.slot /fixture/supervisor_namespace/linked_slot/uid_1000.slot /fixture/supervisor_namespace/writable_slot/uid_1000.slot
chmod 777 /fixture/supervisor_namespace/writable
chown 1000:1000 /fixture/supervisor_namespace/user_owned
ln -s good /fixture/supervisor_namespace/symlink
ln /fixture/supervisor_namespace/linked_slot/uid_1000.slot /fixture/supervisor_namespace/linked_slot/alias
ln -s ../good/uid_1000.slot /fixture/supervisor_namespace/symlink_slot/uid_1000.slot
chmod 666 /fixture/supervisor_namespace/writable_slot/uid_1000.slot
cd /build/terminal-red-7fc3330
setpriv --reuid=1000 --regid=1000 --clear-groups cargo test --offline -p diskgraph-engine --lib linux_supervisor_namespace_tests -- --include-ignored --nocapture' > docs/benchmarks/supervisor_startup_admission_a08/linux_bound_namespace_qualification.log 2>&1

