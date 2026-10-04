//! Git 基线 CAS 的实际归属回归；来源：原生 Rust D34 namespace / EC-02 发布合同。
//! 使用公开注册、staging 和 owned 发布生成合法状态，不写 SQL 伪造归属或 latest。

use crate::git_job_test_fixtures::batch;
use crate::{ControlStore, SqliteSnapshotStore, StoreError};
use diskgraph_core::{
    CollectorBatch, GitEvidenceJobInput, GitEvidenceLimits, Locator, ResourceLocator,
};
use std::path::Path;

/// 两个真实注册 namespace 共用一个根定位的 Store 夹具。
/// 来源：DiskGraph 原生 Rust D34 实际 server/scope 隔离；不是源文件或 Git 采样验收。
struct OwnedBases {
    _directory: tempfile::TempDir,
    store: SqliteSnapshotStore,
    root: ResourceLocator,
    a: GitEvidenceJobInput,
    b: GitEvidenceJobInput,
}

impl OwnedBases {
    fn new() -> Self {
        Self::with_unowned(false)
    }

    fn with_unowned(include_unowned: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("indexed-root");
        std::fs::create_dir_all(path.join("cache")).unwrap();
        let locator = Locator::from_native_path(&path);
        let root = ResourceLocator::NativePath(locator.display.clone());
        let mut control_a = ControlStore::open_in_memory().unwrap();
        let mut control_b = ControlStore::open_in_memory().unwrap();
        let server_a = control_a.ensure_server().unwrap();
        let server_b = control_b.ensure_server().unwrap();
        let scope_a = control_a.register_scope(&locator, None).unwrap();
        let scope_b = control_b.register_scope(&locator, None).unwrap();
        assert_ne!(server_a, server_b);
        assert_ne!(scope_a, scope_b);
        assert_eq!(control_a.scope(&scope_a).unwrap().root, locator);
        assert_eq!(control_b.scope(&scope_b).unwrap().root, locator);
        let a = GitEvidenceJobInput::new(
            server_a,
            scope_a,
            "base-a".into(),
            2,
            GitEvidenceLimits::default(),
        )
        .unwrap();
        let b = GitEvidenceJobInput::new(
            server_b,
            scope_b,
            "base-b".into(),
            2,
            GitEvidenceLimits::default(),
        )
        .unwrap();
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        publish_scan(&mut store, &root, &a, "snapshot-a", "base-a", 100);
        if include_unowned {
            // 真实旧可信 API 的未绑定版本先于 B 发布；B 后来盖住共享指针，历史仍不可猜归属。
            let mut unowned = crate::tests::graph("snapshot-unowned", 150);
            unowned.snapshot.root = root.clone();
            unowned.nodes[0].locator = root.clone();
            unowned.nodes[0].name = "indexed-root".into();
            unowned.nodes[1].locator =
                ResourceLocator::NativePath(path.join("cache").to_string_lossy().into());
            unowned.evidence.clear();
            store
                .append_staging_nodes("scan-unowned", &unowned.nodes)
                .unwrap();
            store
                .publish_revision("scan-unowned", &unowned, "unowned", 150)
                .unwrap();
            assert_eq!(store.staging_node_count("scan-unowned").unwrap(), 0);
        }
        publish_scan(&mut store, &root, &b, "snapshot-b", "base-b", 200);
        assert_eq!(store.revision_ownership("base-a").unwrap(), owner(&a));
        assert_eq!(store.revision_ownership("base-b").unwrap(), owner(&b));
        assert_eq!(
            store.load_revision("base-a").unwrap().snapshot.root,
            store.load_revision("base-b").unwrap().snapshot.root
        );
        Self {
            _directory: directory,
            store,
            root,
            a,
            b,
        }
    }
}

fn owner(input: &GitEvidenceJobInput) -> Option<(String, String)> {
    Some((
        input.server_id().as_str().into(),
        input.scope_id().as_str().into(),
    ))
}

fn publish_scan(
    store: &mut SqliteSnapshotStore,
    root: &ResourceLocator,
    input: &GitEvidenceJobInput,
    snapshot: &str,
    revision: &str,
    at: u64,
) {
    let ResourceLocator::NativePath(path) = root else {
        panic!("该夹具必须使用宿主编码的原生根定位");
    };
    let mut graph = crate::tests::graph(snapshot, at);
    graph.snapshot.root = root.clone();
    graph.nodes[0].locator = root.clone();
    graph.nodes[0].name = "indexed-root".into();
    graph.nodes[1].locator =
        ResourceLocator::NativePath(Path::new(path).join("cache").to_string_lossy().into());
    graph.evidence.clear();
    let staging = format!("scan-{revision}");
    store.append_staging_nodes(&staging, &graph.nodes).unwrap();
    store
        .publish_revision_owned(
            &staging,
            &graph,
            revision,
            at,
            Some((input.server_id().as_str(), input.scope_id().as_str())),
        )
        .unwrap();
    assert_eq!(store.staging_node_count(&staging).unwrap(), 0);
}

fn git_batch(input: &GitEvidenceJobInput, run: &str) -> CollectorBatch {
    let mut result = batch(input, run);
    result.run.snapshot_id = "snapshot-a".into();
    result
}

#[test]
fn another_owner_with_the_same_root_key_does_not_make_the_owned_git_base_stale() {
    let mut fixture = OwnedBases::new();
    // B 的正常公开发布已改共享 root pointer，A 实际归属的最新版本仍是原固定基线。
    assert_eq!(
        fixture
            .store
            .latest_revision_for_root(&fixture.root)
            .unwrap()
            .as_deref(),
        Some("base-b")
    );
    for (input, expected) in [(&fixture.a, "base-a"), (&fixture.b, "base-b")] {
        assert_eq!(
            fixture
                .store
                .latest_revision_for_scope(input.server_id().as_str(), input.scope_id().as_str())
                .unwrap()
                .as_deref(),
            Some(expected)
        );
    }
    let old_a = fixture.store.load_revision("base-a").unwrap();
    let old_b = fixture.store.load_revision("base-b").unwrap();
    let payload = git_batch(&fixture.a, "git-run-a");
    let receipt = fixture
        .store
        .publish_git_collector_revision_checked(
            "git-job-a",
            &fixture.a,
            (7, 300),
            "git-result-a",
            &payload,
            || Ok(()),
        )
        .expect("B 的共享显示根 latest 不能替代 A 实际归属的基线 CAS");
    assert_eq!(receipt.job_id(), "git-job-a");
    assert_eq!(receipt.input(), &fixture.a);
    assert_eq!(receipt.input_sha256(), fixture.a.digest());
    assert_eq!(receipt.server_id(), fixture.a.server_id());
    assert_eq!(receipt.scope_id(), fixture.a.scope_id());
    assert_eq!(receipt.base_revision_id(), "base-a");
    assert_eq!(receipt.snapshot_id(), "snapshot-a");
    assert_eq!(receipt.revision_id(), "git-result-a");
    assert_eq!(receipt.run_id(), "git-run-a");
    assert_eq!(receipt.publishing_fence(), 7);
    assert_eq!(receipt.committed_at_unix_ms(), 300);
    assert_eq!(
        fixture.store.job_publication_receipt("git-job-a").unwrap(),
        Some(receipt)
    );
    assert_eq!(
        fixture.store.revision_ownership("git-result-a").unwrap(),
        owner(&fixture.a)
    );
    assert_eq!(
        fixture
            .store
            .latest_revision_for_scope(
                fixture.a.server_id().as_str(),
                fixture.a.scope_id().as_str()
            )
            .unwrap()
            .as_deref(),
        Some("git-result-a")
    );
    assert_eq!(
        fixture
            .store
            .latest_revision_for_scope(
                fixture.b.server_id().as_str(),
                fixture.b.scope_id().as_str()
            )
            .unwrap()
            .as_deref(),
        Some("base-b")
    );
    assert_eq!(fixture.store.load_revision("base-a").unwrap(), old_a);
    assert_eq!(fixture.store.load_revision("base-b").unwrap(), old_b);
    assert_eq!(fixture.store.load_revision("git-result-a").unwrap(), old_a);
    let selected = fixture.store.revision_evidence("git-result-a").unwrap();
    selected.require_confirmed_membership().unwrap();
    for entity in &payload.entities {
        assert_eq!(
            selected.entity(&entity.entity_id).unwrap(),
            Some(entity.clone())
        );
        for old in ["base-a", "base-b"] {
            assert!(
                fixture
                    .store
                    .revision_evidence(old)
                    .unwrap()
                    .entity(&entity.entity_id)
                    .unwrap()
                    .is_none()
            );
        }
    }
    assert_eq!(selected.all_edges().unwrap(), payload.edges);
    assert_eq!(
        selected
            .evidence_for_edges(&[payload.edges[0].edge_id.clone()])
            .unwrap(),
        payload.evidence
    );
}

#[test]
fn a_new_revision_in_the_actual_owner_scope_rejects_the_fixed_git_base() {
    let mut fixture = OwnedBases::new();
    publish_scan(
        &mut fixture.store,
        &fixture.root,
        &fixture.a,
        "snapshot-a-new",
        "base-a-new",
        400,
    );
    assert_eq!(
        fixture
            .store
            .latest_revision_for_scope(
                fixture.a.server_id().as_str(),
                fixture.a.scope_id().as_str()
            )
            .unwrap()
            .as_deref(),
        Some("base-a-new")
    );
    let before = fixture.store.load_revision("base-a-new").unwrap();
    let payload = git_batch(&fixture.a, "stale-run-a");
    let result = fixture.store.publish_git_collector_revision_checked(
        "stale-job-a",
        &fixture.a,
        (8, 500),
        "rejected-git-a",
        &payload,
        || Ok(()),
    );
    assert!(
        matches!(result, Err(StoreError::Conflict(_))),
        "只有 A 实际归属的新 revision 才使原基线过期：{result:?}"
    );
    assert!(matches!(
        fixture.store.revision("rejected-git-a"),
        Err(StoreError::RevisionNotFound(_))
    ));
    assert!(
        fixture
            .store
            .job_publication_receipt("stale-job-a")
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.store.load_revision("base-a-new").unwrap(), before);
    assert_eq!(
        fixture
            .store
            .latest_revision_for_scope(
                fixture.b.server_id().as_str(),
                fixture.b.scope_id().as_str()
            )
            .unwrap()
            .as_deref(),
        Some("base-b")
    );
    for table in [
        "collector_runs",
        "entities",
        "evidence_records",
        "relations",
        "revision_runs",
        "job_publication_receipts",
    ] {
        let count: i64 = fixture
            .store
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "拒绝的 Git 发布不能残留 {table}");
    }
}

#[test]
fn a_later_unowned_same_root_revision_cannot_be_hidden_by_another_owner_pointer() {
    let mut fixture = OwnedBases::with_unowned(true);
    assert_eq!(fixture.store.revision_ownership("unowned").unwrap(), None);
    assert_eq!(
        fixture
            .store
            .revision("unowned")
            .unwrap()
            .published_at_unix_ms,
        150
    );
    assert_eq!(
        fixture
            .store
            .load_revision("unowned")
            .unwrap()
            .snapshot
            .root,
        fixture.root
    );
    assert_eq!(
        fixture
            .store
            .latest_revision_for_root(&fixture.root)
            .unwrap()
            .as_deref(),
        Some("base-b")
    );
    assert_eq!(
        fixture
            .store
            .latest_revision_for_scope(
                fixture.a.server_id().as_str(),
                fixture.a.scope_id().as_str()
            )
            .unwrap()
            .as_deref(),
        Some("base-a")
    );
    let before_a = fixture.store.load_revision("base-a").unwrap();
    let before_b = fixture.store.load_revision("base-b").unwrap();
    let payload = git_batch(&fixture.a, "unowned-blocked-run");
    let result = fixture.store.publish_git_collector_revision_checked(
        "unowned-blocked-job",
        &fixture.a,
        (9, 300),
        "unowned-blocked-result",
        &payload,
        || Ok(()),
    );
    assert!(
        matches!(result, Err(StoreError::Conflict(_))),
        "未绑定的新历史不能仅因 B 盖过指针而被绕过：{result:?}"
    );
    assert_rejected_result(
        &fixture.store,
        "unowned-blocked-result",
        "unowned-blocked-job",
        3,
    );
    assert_eq!(fixture.store.load_revision("base-a").unwrap(), before_a);
    assert_eq!(fixture.store.load_revision("base-b").unwrap(), before_b);
    assert_eq!(
        fixture
            .store
            .latest_revision_for_root(&fixture.root)
            .unwrap()
            .as_deref(),
        Some("base-b")
    );
}

#[test]
fn a_backdated_git_result_that_cannot_be_owner_latest_rolls_back_every_publication_row() {
    let mut fixture = OwnedBases::new();
    // 此案无未绑定历史；A 的 owned 最新版本、节点和批次均合法，只有新发布时间倒退。
    let unowned: i64 = fixture.store.connection.query_row(
        "SELECT COUNT(*) FROM graph_revisions r LEFT JOIN revision_ownership o ON o.revision_id=r.revision_id WHERE o.revision_id IS NULL",
        [], |row| row.get(0)
    ).unwrap();
    assert_eq!(unowned, 0);
    assert_eq!(
        fixture
            .store
            .revision("base-a")
            .unwrap()
            .published_at_unix_ms,
        100
    );
    assert_eq!(
        fixture
            .store
            .latest_revision_for_scope(
                fixture.a.server_id().as_str(),
                fixture.a.scope_id().as_str()
            )
            .unwrap()
            .as_deref(),
        Some("base-a")
    );
    assert_eq!(
        fixture
            .store
            .latest_revision_for_root(&fixture.root)
            .unwrap()
            .as_deref(),
        Some("base-b")
    );
    let before_a = fixture.store.load_revision("base-a").unwrap();
    let before_b = fixture.store.load_revision("base-b").unwrap();
    let payload = git_batch(&fixture.a, "backdated-run");
    let result = fixture.store.publish_git_collector_revision_checked(
        "backdated-job",
        &fixture.a,
        (10, 99),
        "backdated-result",
        &payload,
        || Ok(()),
    );
    assert!(
        matches!(result, Err(StoreError::Conflict(_))),
        "不能成为同 owner 最新历史的 Git 结果不得提交：{result:?}"
    );
    assert_rejected_result(&fixture.store, "backdated-result", "backdated-job", 2);
    assert_eq!(fixture.store.load_revision("base-a").unwrap(), before_a);
    assert_eq!(fixture.store.load_revision("base-b").unwrap(), before_b);
    assert_eq!(
        fixture
            .store
            .latest_revision_for_root(&fixture.root)
            .unwrap()
            .as_deref(),
        Some("base-b")
    );
}

fn assert_rejected_result(store: &SqliteSnapshotStore, revision: &str, job: &str, revisions: i64) {
    assert!(matches!(
        store.revision(revision),
        Err(StoreError::RevisionNotFound(_))
    ));
    assert_eq!(store.revision_ownership(revision).unwrap(), None);
    assert!(store.job_publication_receipt(job).unwrap().is_none());
    let count: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM graph_revisions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, revisions);
    for table in [
        "collector_runs",
        "entities",
        "evidence_records",
        "relations",
        "entity_run_memberships",
        "relation_run_memberships",
        "revision_runs",
        "job_publication_receipts",
    ] {
        let count: i64 = store
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "完整回滚不得残留 {table}");
    }
}
