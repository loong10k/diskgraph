//! 原授权路径祖先绑定负控；来源：真实 tmpfs rename，叶目录/文件本身身份保持不变。
use super::ProcessNativeSession;
use super::linux_proc_root::LinuxProcRoot;
use super::linux_target::LinuxTarget;
use crate::process_execution_fixture::ProcessExecutionFixture;
use diskgraph_core::{ProcessEvidenceFailureCode as Failure, ProcessEvidenceLimits};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

#[test]
fn replaced_ancestor_with_original_leaf_moved_back_cannot_pass_terminal_binding() {
    let f = ProcessExecutionFixture::new();
    let input = f.input(&f.base, ProcessEvidenceLimits::default());
    let cancel = AtomicBool::new(false);
    let check = || Ok(());
    let session =
        ProcessNativeSession::new(input.limits(), Instant::now(), &cancel, &check).unwrap();
    let procfs = LinuxProcRoot::open(&|| session.check()).unwrap();
    let root = f.source.path().join("container/scope");
    let held = LinuxTarget::open(
        &root,
        Path::new("target"),
        input.indexed_epoch(),
        &procfs.boot,
        &session,
    )
    .unwrap();
    // 不换叶对象：把原 scope 放回同一拼写，但其原 container 已换成另一个目录。
    std::fs::rename(
        f.source.path().join("container"),
        f.source.path().join("original-container"),
    )
    .unwrap();
    std::fs::create_dir(f.source.path().join("container")).unwrap();
    std::fs::rename(f.source.path().join("original-container/scope"), &root).unwrap();
    let result = held.verify(input.indexed_epoch(), &procfs.boot, &session);
    assert_eq!(
        result,
        Err(Failure::Conflict),
        "terminal must retain the original ancestor route even when the leaf object survives"
    );
}

#[test]
fn unrelated_sibling_creation_does_not_invalidate_original_route_identity() {
    let f = ProcessExecutionFixture::new();
    let input = f.input(&f.base, ProcessEvidenceLimits::default());
    let cancel = AtomicBool::new(false);
    let check = || Ok(());
    let session =
        ProcessNativeSession::new(input.limits(), Instant::now(), &cancel, &check).unwrap();
    let procfs = LinuxProcRoot::open(&|| session.check()).unwrap();
    let root = f.source.path().join("container/scope");
    let held = LinuxTarget::open(
        &root,
        Path::new("target"),
        input.indexed_epoch(),
        &procfs.boot,
        &session,
    )
    .unwrap();
    std::fs::write(
        f.source.path().join("unrelated-sibling"),
        b"not part of the captured resource",
    )
    .unwrap();
    held.verify(input.indexed_epoch(), &procfs.boot, &session)
        .unwrap();
}
