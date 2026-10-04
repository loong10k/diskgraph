//! Linux 扫描补充的原祖先绑定验收；来源：FS-02，真实 tmpfs 和公开 Engine 索引。
//! 缺少原生 epoch 能力必须失败并报告未验收，不以其他平台 cfg 零测试当作通过。
use super::BEFORE_STAGE_LOCK;
use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::{
    BusinessError, IndexedFileEpoch, PrincipalId, QueryBudget, QueryReadBudget, ScopeId,
};
use diskgraph_store::{JobRecord, JobState};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 先完成实际扫描证明原生资格，再在同根第二扫描编码后改变 namespace；来源：Rust FS-02。
struct ScanFixture {
    source: tempfile::TempDir,
    data: tempfile::TempDir,
    engine: Arc<Engine>,
    actor: PrincipalId,
    scope: ScopeId,
    base: String,
    epoch: IndexedFileEpoch,
}
impl ScanFixture {
    fn new() -> Self {
        let source = tempfile::tempdir_in("/dev/shm")
            .expect("Linux native prerequisite: writable tmpfs required, not a skipped pass");
        let data = tempfile::tempdir().unwrap();
        let root = source.path().join("container/scope");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("file"), b"ordinary native identity fixture").unwrap();
        let mut config = EngineConfig {
            data_dir: data.path().to_owned(),
            ..EngineConfig::default()
        };
        // 根和唯一文件全部进入同一批次，现有 before_stage_lock 位于两者补充采样和编码成本之后。
        config.scan_budget.write_batch_nodes = 32;
        let engine = Arc::new(Engine::open(config).unwrap());
        let actor = PrincipalId::new("scan-namespace-fixture").unwrap();
        engine.bootstrap_local_admin(&actor).unwrap();
        let scope = engine
            .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
            .unwrap();
        let auth = engine.policy_authorizer().unwrap();
        let scan = engine.index_scope(&scope, &actor, &auth).unwrap();
        assert_eq!(
            engine
                .run_job(&scan.job_id, "baseline-index")
                .unwrap()
                .state,
            JobState::Completed
        );
        let base = engine
            .revision_for_job(&scan.job_id, &actor, &auth)
            .unwrap();
        let epoch = captured_epoch(&engine, &base);
        Self {
            source,
            data,
            engine,
            actor,
            scope,
            base,
            epoch,
        }
    }
    fn root(&self) -> PathBuf {
        self.source.path().join("container/scope")
    }
    fn enqueue(&self) -> JobRecord {
        self.engine
            .index_scope(
                &self.scope,
                &self.actor,
                &self.engine.policy_authorizer().unwrap(),
            )
            .unwrap()
    }
    fn formal_counts(&self) -> Vec<(&'static str, i64)> {
        let db = rusqlite::Connection::open_with_flags(
            self.data.path().join("diskgraph.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        [
            "snapshots",
            "nodes",
            "node_unix_observations",
            "graph_revisions",
            "collector_runs",
            "entities",
            "evidence_records",
            "relations",
            "revision_runs",
            "relation_run_memberships",
            "entity_run_memberships",
        ]
        .into_iter()
        .map(|table| {
            let count = db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            (table, count)
        })
        .collect()
    }
    fn assert_clean(&self, id: &str, fence: u64) {
        let db = rusqlite::Connection::open_with_flags(
            self.data.path().join("diskgraph.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let staging = format!("{id}:{fence}");
        for table in [
            "scan_staging",
            "scan_staging_search",
            "scan_staging_unix_observations",
        ] {
            let count: i64 = db
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE job_id=?1"),
                    [&staging],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "staging residue: {table}");
        }
        assert!(!self.engine.cancellations.lock().unwrap().contains_key(id));
    }
}
fn captured_epoch(engine: &Engine, revision: &str) -> IndexedFileEpoch {
    let node = engine
        .revision_node_at(revision, Path::new("file"))
        .unwrap()
        .unwrap();
    let reader = engine.revision_reader().unwrap();
    let snapshot = reader.revision(revision).unwrap().snapshot_id;
    let mut reads = QueryReadBudget::new(
        QueryBudget::default(),
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    let saved = reader
        .unix_observation_bounded(&snapshot, node.id, &mut reads)
        .unwrap()
        .expect("actual Linux scan must persist the native observation");
    assert!(
        saved.gap.is_none(),
        "native prerequisites not verified; cannot count as product RED or pass: {:?}",
        saved.gap
    );
    let observation = saved
        .observation
        .expect("supported native fixture requires captured epoch");
    observation.validate().unwrap();
    assert!(matches!(
        observation.epoch(),
        IndexedFileEpoch::LinuxHandle { .. }
    ));
    observation.epoch().clone()
}
fn identity(path: &Path) -> (u64, u64) {
    let metadata = std::fs::symlink_metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap()
}
fn run_changed_namespace(replace_ancestor: bool) {
    let f = ScanFixture::new();
    let before = f.formal_counts();
    assert_eq!(
        before
            .iter()
            .find(|(table, _)| *table == "nodes")
            .unwrap()
            .1,
        2,
        "root and file must fit one observed batch"
    );
    let job = f.enqueue();
    let root = f.root();
    let container = f.source.path().join("container");
    let retained = f.source.path().join("retained-container");
    let parent_identity = identity(&container);
    let root_identity = identity(&root);
    let file_identity = identity(&root.join("file"));
    let witness = Arc::new(Mutex::new(None::<JobRecord>));
    let saved_witness = Arc::clone(&witness);
    let engine = Arc::clone(&f.engine);
    let job_id = job.job_id.clone();
    BEFORE_STAGE_LOCK.with(|slot| {
        assert!(slot.borrow().is_none(), "request hook must start empty");
        *slot.borrow_mut() = Some((
            job.job_id.clone(),
            Box::new(move || {
                let active = engine.job_status(&job_id).unwrap();
                assert_eq!(active.state, JobState::Running);
                assert_eq!(active.owner, "namespace-index");
                assert!(active.fencing_token > 0);
                assert!(active.lease_expires_unix_ms > now_ms());
                assert_eq!(identity(&root), root_identity);
                assert_eq!(identity(&root.join("file")), file_identity);
                if replace_ancestor {
                    std::fs::rename(&container, &retained).unwrap();
                    std::fs::create_dir(&container).unwrap();
                    std::fs::rename(retained.join("scope"), &root).unwrap();
                    assert_ne!(identity(&container), parent_identity);
                    assert_eq!(identity(&retained), parent_identity);
                } else {
                    // 只改变 scope 外祖先的旁支和其目录时间；扫描范围内没有新增或丢失节点。
                    let sibling = container.join("unrelated-sibling");
                    std::fs::write(&sibling, b"outside registered scope").unwrap();
                    std::fs::remove_file(&sibling).unwrap();
                    assert_eq!(identity(&container), parent_identity);
                }
                assert_eq!(
                    identity(&root),
                    root_identity,
                    "original root moved back, not a replacement leaf"
                );
                assert_eq!(identity(&root.join("file")), file_identity);
                *saved_witness.lock().unwrap() = Some(active);
            }),
        ));
    });
    let result = f.engine.run_job(&job.job_id, "namespace-index");
    let pending = BEFORE_STAGE_LOCK.with(|slot| slot.borrow_mut().take());
    assert!(
        pending.is_none(),
        "real post-observation/pre-staging hook was not consumed: {result:?}"
    );
    let witnessed = witness
        .lock()
        .unwrap()
        .take()
        .expect("actual namespace mutation must finish before the assertion");
    let terminal = f.engine.job_status(&job.job_id).unwrap();
    assert_eq!(terminal.owner, witnessed.owner);
    assert_eq!(terminal.fencing_token, witnessed.fencing_token);
    assert!(
        terminal.lease_expires_unix_ms > now_ms(),
        "owner expiry cannot substitute for namespace refusal"
    );
    if replace_ancestor {
        assert!(
            matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
            "changed original ancestor must fail even when final root inode is unchanged: {result:?}"
        );
        assert_eq!(terminal.state, JobState::Failed);
        assert_eq!(
            f.engine.latest_revision(&f.scope).unwrap().as_deref(),
            Some(f.base.as_str())
        );
        assert_eq!(
            f.formal_counts(),
            before,
            "failed scan must not leave formal graph rows"
        );
        assert_eq!(captured_epoch(&f.engine, &f.base), f.epoch);
    } else {
        assert_eq!(result.unwrap().state, JobState::Completed);
        assert_eq!(terminal.state, JobState::Completed);
        let revision = f
            .engine
            .revision_for_job(
                &job.job_id,
                &f.actor,
                &f.engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        assert_ne!(revision, f.base);
        assert_eq!(
            f.engine.latest_revision(&f.scope).unwrap().as_deref(),
            Some(revision.as_str())
        );
        assert_eq!(captured_epoch(&f.engine, &revision), f.epoch);
    }
    f.assert_clean(&job.job_id, witnessed.fencing_token);
}
#[test]
fn linux_scan_rejects_rebound_ancestor_with_original_root_and_file_unchanged() {
    run_changed_namespace(true);
}
#[test]
fn linux_scan_allows_unrelated_ancestor_sibling_changes() {
    run_changed_namespace(false);
}
