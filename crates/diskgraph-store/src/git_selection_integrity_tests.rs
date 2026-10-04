//! Git 选择在外部损坏后的来源完整性回归；来源：原生 Rust EV-05 / EC-02。
//! 正常状态全部通过公开发布生成；仅损坏步骤用独立 FK OFF 连接删除 run。

use crate::git_job_test_fixtures::batch;
use crate::{ControlStore, SqliteSnapshotStore, StoreError};
use diskgraph_core::{
    GitEvidenceJobInput, GitEvidenceLimits, Locator, QueryBudget, QueryReadBudget, ResourceLocator,
};
use rusqlite::Connection;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 已公开发布并封存一份完整旧选择的独占磁盘夹具。
/// 来源：DiskGraph 原生 Rust EV-05 来源闭包；不是远程认证或 Git 源采样夹具。
struct SelectionFixture {
    _directory: tempfile::TempDir,
    database: PathBuf,
    store: SqliteSnapshotStore,
    input: GitEvidenceJobInput,
    root: ResourceLocator,
}

impl SelectionFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("indexed-root");
        std::fs::create_dir_all(path.join("cache")).unwrap();
        let locator = Locator::from_native_path(&path);
        let root = ResourceLocator::NativePath(locator.display.clone());
        let mut control = ControlStore::open_in_memory().unwrap();
        let server = control.ensure_server().unwrap();
        let scope = control.register_scope(&locator, None).unwrap();
        assert_eq!(control.scope(&scope).unwrap().root, locator);
        let database = directory.path().join("graph.sqlite");
        let mut store = SqliteSnapshotStore::open(&database).unwrap();
        let mut graph = crate::tests::graph("snapshot", 100);
        graph.snapshot.root = root.clone();
        graph.nodes[0].locator = root.clone();
        graph.nodes[0].name = "indexed-root".into();
        graph.nodes[1].locator =
            ResourceLocator::NativePath(path.join("cache").to_string_lossy().into());
        graph.evidence.clear();
        store.append_staging_nodes("initial", &graph.nodes).unwrap();
        store
            .publish_revision_owned(
                "initial",
                &graph,
                "base",
                100,
                Some((server.as_str(), scope.as_str())),
            )
            .unwrap();
        store
            .publish_collector_revision(
                "base",
                "old-revision",
                1001,
                (server.as_str(), scope.as_str()),
                &crate::collector_publication_tests::batch("old", "snapshot"),
                &[("old", "active")],
            )
            .unwrap();
        let input = GitEvidenceJobInput::new(
            server,
            scope,
            "old-revision".into(),
            2,
            GitEvidenceLimits::default(),
        )
        .unwrap();
        let fixture = Self {
            _directory: directory,
            database,
            store,
            input,
            root,
        };
        assert_eq!(fixture.flags(), (1, 1));
        assert_eq!(fixture.old_role(), "active");
        assert_eq!(fixture.count("collector_runs"), 1);
        fixture
    }

    fn flags(&self) -> (i64, i64) {
        self.store
            .connection
            .query_row(
                "SELECT selection_sealed,evidence_complete FROM graph_revisions
                 WHERE revision_id='old-revision'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    fn old_role(&self) -> String {
        self.store
            .connection
            .query_row(
                "SELECT role FROM revision_runs WHERE revision_id='old-revision' AND run_id='old'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn count(&self, table: &str) -> i64 {
        self.store
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn selection(&self, revision: &str) -> crate::Result<Vec<(String, String)>> {
        let mut reads = QueryReadBudget::new(
            QueryBudget {
                max_nodes: 1,
                max_edges: 32,
                max_response_bytes: 1 << 20,
                ..QueryBudget::default()
            },
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        self.store
            .git_collector_selection_with_budget(revision, 2, &mut reads)
    }

    fn publish(&mut self) -> crate::Result<diskgraph_core::JobPublicationReceipt> {
        self.store.publish_git_collector_revision_checked(
            "job-new",
            &self.input,
            (1, 1002),
            "new-revision",
            &batch(&self.input, "new"),
            || Ok(()),
        )
    }
}

#[test]
fn a_complete_selected_run_is_preserved_by_normal_git_publication() {
    let mut fixture = SelectionFixture::new();
    assert_eq!(
        fixture.selection("old-revision").unwrap(),
        vec![("old".into(), "active".into())]
    );
    let receipt = fixture.publish().unwrap();
    assert_eq!(receipt.revision_id(), "new-revision");
    assert_eq!(
        fixture.selection("new-revision").unwrap(),
        vec![
            ("new".into(), "dependency_only".into()),
            ("old".into(), "active".into())
        ]
    );
    assert_eq!(fixture.old_role(), "active");
    assert_eq!(fixture.flags(), (1, 1));
    assert_eq!(fixture.count("collector_runs"), 2);
    assert_eq!(fixture.count("job_publication_receipts"), 1);
}

#[test]
fn a_dangling_selected_run_cannot_be_laundered_into_a_complete_git_revision() {
    let mut fixture = SelectionFixture::new();
    assert_eq!(
        fixture.selection("old-revision").unwrap(),
        vec![("old".into(), "active".into())]
    );
    // 明确模拟外连关闭 FK 后产生的损坏，不声称正常公开 API 能删除受引用 run。
    {
        let external = Connection::open(&fixture.database).unwrap();
        external.pragma_update(None, "foreign_keys", "OFF").unwrap();
        assert_eq!(
            external
                .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            external
                .execute("DELETE FROM collector_runs WHERE run_id='old'", [])
                .unwrap(),
            1
        );
    }
    assert_eq!(fixture.count("collector_runs"), 0);
    assert_eq!(fixture.old_role(), "active");
    assert_eq!(fixture.flags(), (1, 1));
    let tables = [
        "snapshots",
        "nodes",
        "graph_revisions",
        "revision_ownership",
        "collector_runs",
        "revision_runs",
        "entities",
        "relations",
        "evidence_records",
        "entity_run_memberships",
        "relation_run_memberships",
        "job_publication_receipts",
    ];
    let before = tables.map(|table| fixture.count(table));
    let result = fixture.publish();
    assert!(
        matches!(&result, Err(StoreError::InvalidGraph(_))),
        "damaged selected source must fail closed before publication: {result:?}"
    );
    assert!(
        matches!(
            fixture.selection("old-revision"),
            Err(StoreError::InvalidGraph(_))
        ),
        "the public complete-selection reader must also reject the dangling source"
    );
    for (table, expected) in tables.into_iter().zip(before) {
        assert_eq!(fixture.count(table), expected, "rollback of {table}");
    }
    assert!(
        fixture
            .store
            .job_publication_receipt("job-new")
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        fixture.store.revision("new-revision"),
        Err(StoreError::RevisionNotFound(_))
    ));
    assert_eq!(fixture.old_role(), "active");
    assert_eq!(fixture.flags(), (1, 1));
    assert_eq!(
        fixture
            .store
            .latest_revision_for_root(&fixture.root)
            .unwrap()
            .as_deref(),
        Some("old-revision")
    );
    assert_eq!(
        fixture
            .store
            .latest_revision_for_scope(
                fixture.input.server_id().as_str(),
                fixture.input.scope_id().as_str()
            )
            .unwrap()
            .as_deref(),
        Some("old-revision")
    );
}
