//! 计划的实际读取范围、旧行兼容与错误顺序回归；只使用隔离数据库和文件。

use super::{indexed, node_named, project, tree};
use crate::{OpsError, PlanBuilder, plan_digest};
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;
use diskgraph_store::StoreError;
use rusqlite::Connection;

mod plan_query_benchmark;

fn graph_connection(project: &super::Project) -> Connection {
    Connection::open(project._workspace.path().join("data/diskgraph.sqlite")).unwrap()
}

fn persisted_plan_count(project: &super::Project) -> i64 {
    Connection::open(
        project
            ._workspace
            .path()
            .join("data/diskgraph-control.sqlite"),
    )
    .unwrap()
    .query_row("SELECT count(*) FROM plans", [], |row| row.get(0))
    .unwrap()
}

#[test]
fn plans_decode_only_selected_rows_and_preserve_the_exact_items() {
    let mut project = project("narrow-selection");
    let (scope, principal) = indexed(&mut project);
    let revision = project.engine.latest_revision(&scope).unwrap().unwrap();
    let app = node_named(&project.engine, &scope, "app.bin");
    let deep = node_named(&project.engine, &scope, "deep.bin");
    let before = tree(&project.root);
    let before_count = persisted_plan_count(&project);
    let connection = graph_connection(&project);
    // 破坏所有未选行；完整加载必须失败，按选择读取仍必须构造正确计划。
    let changed = connection
        .execute(
            "UPDATE nodes SET kind='invalid-kind' WHERE id NOT IN (?1,?2)",
            [i64::try_from(app).unwrap(), i64::try_from(deep).unwrap()],
        )
        .unwrap();
    assert!(changed > 0);
    assert!(project.engine.load_revision(&revision).is_err());
    let plan = PlanBuilder::new(project.engine.clone())
        .build_trash_plan(&scope, &principal, &[app, deep], 1 << 20)
        .expect("unselected rows must not be decoded");
    let mut actual: Vec<_> = plan.items.iter().map(|item| item.node_id).collect();
    actual.sort_unstable();
    let mut expected = vec![app, deep];
    expected.sort_unstable();
    assert_eq!(actual, expected);
    assert_eq!(plan.expected_bytes, 5120);
    let control = project.engine.control_store().unwrap();
    assert_eq!(control.plan(&plan.plan_id).unwrap(), plan);
    assert_eq!(
        control.plan_digest(&plan.plan_id).unwrap(),
        plan_digest(&plan)
    );
    assert_eq!(tree(&project.root), before);
    assert_eq!(persisted_plan_count(&project), before_count + 1);
}

#[test]
fn selected_legacy_unknown_size_rows_keep_their_live_evidence() {
    let mut project = project("narrow-legacy");
    let (scope, principal) = indexed(&mut project);
    let revision = project.engine.latest_revision(&scope).unwrap().unwrap();
    let graph = project.engine.load_revision(&revision).unwrap();
    let mut node = graph
        .nodes
        .iter()
        .find(|node| node.name == "app.bin")
        .unwrap()
        .clone();
    node.size_known = false;
    let connection = graph_connection(&project);
    connection
        .execute(
            "UPDATE nodes SET kind=NULL,direct_bytes=NULL,files=NULL,directories=NULL,
         modified_unix_seconds=NULL,file_volume_id=NULL,file_id=NULL,read_error=NULL,
         node_json=?1 WHERE id=?2",
            rusqlite::params![
                serde_json::to_string(&node).unwrap(),
                i64::try_from(node.id).unwrap()
            ],
        )
        .unwrap();
    let plan = PlanBuilder::new(project.engine.clone())
        .build_trash_plan(&scope, &principal, &[node.id], 1 << 20)
        .unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].node_id, node.id);
    assert!(plan.items[0].source_fingerprint.is_some());
    assert_eq!(plan.expected_bytes, 4096);
}

#[test]
fn a_selected_malformed_column_fails_before_plan_persistence() {
    let mut project = project("narrow-invalid-selected");
    let (scope, principal) = indexed(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let before = tree(&project.root);
    let before_count = persisted_plan_count(&project);
    let connection = graph_connection(&project);
    connection
        .execute(
            "UPDATE nodes SET direct_bytes='invalid' WHERE id=?1",
            [i64::try_from(app).unwrap()],
        )
        .unwrap();
    assert!(matches!(
        PlanBuilder::new(project.engine.clone()).build_trash_plan(
            &scope,
            &principal,
            &[app],
            1 << 20
        ),
        Err(OpsError::Engine(EngineError::Store(StoreError::Sqlite(_))))
    ));
    assert_eq!(tree(&project.root), before);
    assert_eq!(persisted_plan_count(&project), before_count);
}

#[test]
fn a_cross_wired_root_latest_pointer_cannot_replace_scope_ownership() {
    let mut project = project("narrow-owner");
    let (scope, principal) = indexed(&mut project);
    let original = project.engine.latest_revision(&scope).unwrap().unwrap();
    let app = node_named(&project.engine, &scope, "app.bin");
    let foreign_root = project._workspace.path().join("other-scope");
    std::fs::create_dir_all(&foreign_root).unwrap();
    std::fs::write(foreign_root.join("foreign.bin"), b"foreign").unwrap();
    let authorizer = project.engine.policy_authorizer().unwrap();
    let foreign_scope = project
        .engine
        .register_scope(&foreign_root, &principal, &authorizer)
        .unwrap();
    let authorizer = project.engine.policy_authorizer().unwrap();
    let job = project
        .engine
        .index_scope(&foreign_scope, &principal, &authorizer)
        .unwrap();
    project.engine.run_job(&job.job_id, "narrow-owner").unwrap();
    let foreign_revision = project
        .engine
        .latest_revision(&foreign_scope)
        .unwrap()
        .unwrap();
    let authorizer = project.engine.policy_authorizer().unwrap();
    assert!(
        project
            .engine
            .authorize_revision(None, &foreign_revision, &principal, &authorizer)
            .is_ok()
    );
    graph_connection(&project)
        .execute(
            "UPDATE latest_revision SET revision_id=?1 WHERE revision_id=?2",
            [&foreign_revision, &original],
        )
        .unwrap();
    // Engine 的 latest 以持久归属查询；旧 root 指针被串接也不能替代 A 的 revision。
    assert_eq!(
        project.engine.latest_revision(&scope).unwrap(),
        Some(original)
    );
    let before_count = persisted_plan_count(&project);
    let plan = PlanBuilder::new(project.engine.clone())
        .build_trash_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();
    assert_eq!(plan.scope_id, scope);
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].node_id, app);
    assert_eq!(plan.expected_bytes, 4096);
    assert_eq!(persisted_plan_count(&project), before_count + 1);
    assert_eq!(
        std::fs::read(foreign_root.join("foreign.bin")).unwrap(),
        b"foreign"
    );
}

#[test]
fn missing_ids_outside_sqlite_range_remain_missing_nodes() {
    let mut project = project("narrow-large-id");
    let (scope, principal) = indexed(&mut project);
    let builder = PlanBuilder::new(project.engine.clone());
    for id in [i64::MAX as u64, i64::MAX as u64 + 1, u64::MAX] {
        assert!(
            matches!(builder.build_trash_plan(&scope, &principal, &[id], 1 << 20),
            Err(OpsError::NoSuchNode(actual)) if actual == id)
        );
    }
}

#[test]
fn a_stale_earlier_path_is_not_hidden_by_a_later_missing_node() {
    let mut project = project("narrow-order");
    let (scope, principal) = indexed(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    std::fs::remove_file(project.root.join("target/app.bin")).unwrap();
    let builder = PlanBuilder::new(project.engine.clone());
    assert!(matches!(
        builder.build_trash_plan(&scope, &principal, &[app, 999_999], 1 << 20),
        Err(OpsError::Stale(_))
    ));
    assert!(matches!(
        builder.build_trash_plan(&scope, &principal, &[999_999, app], 1 << 20),
        Err(OpsError::NoSuchNode(999_999))
    ));
}

#[test]
fn final_authorization_lock_wait_cannot_extend_the_metadata_deadline() {
    let mut project = project("narrow-final-deadline");
    let (scope, principal) = indexed(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let before_count = persisted_plan_count(&project);
    let before = tree(&project.root);
    let threads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let owned_threads = threads.clone();
    let engine = project.engine.clone();
    let mut builder = PlanBuilder::new(project.engine.clone());
    // 在真实 consumer 结束前，另一 owner 已持有控制锁；helper 的最终授权将等待。
    builder.metadata_read_observer = Some(Box::new(move || {
        let (locked, observed) = std::sync::mpsc::channel();
        let engine = engine.clone();
        let owner = std::thread::spawn(move || {
            let _guard = engine.control_store().unwrap();
            locked.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100));
        });
        observed
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        owned_threads.lock().unwrap().push(owner);
    }));
    assert!(matches!(
        builder.build_trash_plan(&scope, &principal, &[app], 1 << 20),
        Err(OpsError::Engine(EngineError::Business(
            BusinessError::BudgetExceeded
        )))
    ));
    for owner in threads.lock().unwrap().drain(..) {
        owner.join().unwrap();
    }
    assert_eq!(persisted_plan_count(&project), before_count);
    assert_eq!(tree(&project.root), before);
}
