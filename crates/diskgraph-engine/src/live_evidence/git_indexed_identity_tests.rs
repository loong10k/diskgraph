//! D38 索引目录身份绑定前置；来源：实际扫描持久身份与真实 Git 目录替换。
//! 初始真实 RED 由可信 scoped 入口复现当前路径稳定仍可采到替换目录。
//! 本回归现使用实际 revision 派生身份的产品采集边界，旧可信入口语义保持。

use super::git_isolation_fixture::GitIsolationFixture;
use super::{EvidenceProbeSession, ProbeLimits};
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use crate::Engine;
use crate::EngineConfig;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use diskgraph_core::{NodeKind, PrincipalId, QueryBudget};
use std::path::Path;

#[test]
fn indexed_git_directory_replacement_cannot_be_sampled_as_the_original_node() {
    let original = GitIsolationFixture::new("sha1");
    let replacement = GitIsolationFixture::new("sha1");
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: data.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = PrincipalId::new("indexed-git-fixture").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let scope = engine
        .register_scope(original.path(), &actor, &policy)
        .unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "indexed-git-fixture").unwrap();
    let revision = engine
        .revision_for_job(&job.job_id, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let indexed = engine.revision_root_node(&revision).unwrap();
    assert_eq!(indexed.kind, NodeKind::Directory);
    let locator = engine
        .revision_node_locator(
            &revision,
            indexed.id,
            &actor,
            &engine.policy_authorizer().unwrap(),
            QueryBudget::default(),
        )
        .unwrap()
        .locator
        .unwrap();
    #[cfg(windows)]
    let windows = engine
        .revision_windows_observation(
            &revision,
            indexed.id,
            &actor,
            &engine.policy_authorizer().unwrap(),
            QueryBudget::default(),
        )
        .unwrap()
        .observation;
    #[cfg(not(windows))]
    let windows = None;
    let expected = super::GitIndexedDirectory::from_node(&indexed, windows.as_ref()).unwrap();
    let mut positive = EvidenceProbeSession::new(&ProbeLimits::default()).unwrap();
    let clean = positive
        .sample_git_indexed(Path::new("git"), original.path(), &locator, &expected)
        .unwrap();
    assert_eq!(
        clean.dirty_count, 0,
        "unchanged actual indexed repository positive control"
    );

    // 同一目录内正文变化是采样目的，不能用目录 mtime 把正常 dirty 状态拒绝。
    std::fs::write(original.path().join("tracked"), b"normal indexed edit\n").unwrap();
    let mut changed = EvidenceProbeSession::new(&ProbeLimits::default()).unwrap();
    assert_eq!(
        changed
            .sample_git_indexed(Path::new("git"), original.path(), &locator, &expected)
            .unwrap()
            .dirty_count,
        1
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let observed = indexed.file_identity.as_ref().unwrap();
        let old = std::fs::metadata(original.path()).unwrap();
        assert_eq!(observed.file_id, old.ino());
        assert_eq!(observed.volume_id, old.dev().to_string());
        let other = std::fs::metadata(replacement.path()).unwrap();
        assert_ne!((old.dev(), old.ino()), (other.dev(), other.ino()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_READ_ATTRIBUTES,
        };
        let observed = engine
            .revision_windows_observation(
                &revision,
                indexed.id,
                &actor,
                &engine.policy_authorizer().unwrap(),
                QueryBudget::default(),
            )
            .unwrap()
            .observation
            .unwrap();
        let other = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .custom_flags(
                FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_NO_RECALL
                    | FILE_FLAG_OPEN_REPARSE_POINT,
            )
            .open(replacement.path())
            .unwrap();
        let other = crate::windows_file_state::WindowsFileState::capture(&other)
            .unwrap()
            .observation(0, 0, diskgraph_core::WindowsTreeAlignment::Matched);
        assert_ne!(
            (observed.volume, observed.file_id),
            (other.volume, other.file_id)
        );
    }
    let retained_original = original.path().parent().unwrap().join("indexed-original");
    std::fs::rename(original.path(), &retained_original).unwrap();
    std::fs::rename(replacement.path(), original.path()).unwrap();
    std::fs::write(
        original.path().join("tracked"),
        b"different live repository\n",
    )
    .unwrap();
    let native = original.git(&["status", "--porcelain=v1", "-z", "--untracked-files=all"]);
    assert_eq!(
        native, b" M tracked\0",
        "replacement is a real, stable dirty repository"
    );
    let mut session = EvidenceProbeSession::new(&ProbeLimits::default()).unwrap();
    let result = session.sample_git_indexed(Path::new("git"), original.path(), &locator, &expected);
    eprintln!("indexed directory replaced before capture; actual scoped result={result:?}");
    assert!(
        result.is_err(),
        "a stable replacement must not become evidence for the original indexed node: {result:?}"
    );
}
