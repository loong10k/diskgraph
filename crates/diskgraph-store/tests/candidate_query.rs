use diskgraph_core::{
    DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, EvidenceRelation, NodeKind, QueryBudget,
    ResourceLocator, ScanCoverage, ScanSettings, TruncationReason,
};
use diskgraph_store::{CandidateSelection, SqliteSnapshotStore, StoreError};

fn node(id: u64, parent_id: Option<u64>, name: &str, bytes: u64, kind: NodeKind) -> DiskNode {
    let path = match id {
        1 => "/fixture".to_owned(),
        3 => "/fixture/blocked-dir/protected-file".to_owned(),
        _ => format!("/fixture/{name}"),
    };
    DiskNode {
        id,
        parent_id,
        locator: ResourceLocator::NativePath(path),
        name: name.into(),
        kind,
        subtree_bytes: bytes,
        direct_bytes: if kind == NodeKind::File { bytes } else { 0 },
        size_known: true,
        files: 1,
        directories: usize::from(kind == NodeKind::Directory) as u64,
        modified_unix_seconds: None,
        file_identity: None,
        category_hint: None,
        reclaim_hint: None,
        read_error: false,
    }
}

fn evidence(node_id: u64, relation: EvidenceRelation) -> EvidenceEdge {
    EvidenceEdge {
        node_id,
        relation,
        subject: "fixture".into(),
        source: "test".into(),
        observed_at_unix_ms: 1,
        confidence: 100,
    }
}

fn graph() -> DiskGraph {
    DiskGraph {
        snapshot: DiskSnapshot {
            id: "candidate-snapshot".into(),
            root: ResourceLocator::NativePath("/fixture".into()),
            volume_id: Some("fixture-volume".into()),
            captured_at_unix_ms: 1,
            settings: ScanSettings {
                apparent_size: true,
                follow_links: false,
                include_hidden: true,
                one_filesystem: true,
                max_depth: None,
                dedup_hardlinks: true,
            },
            coverage: ScanCoverage {
                complete: true,
                unreadable_nodes: 0,
                depth_limited: false,
            },
        },
        nodes: vec![
            node(1, None, "root", 1_000, NodeKind::Directory),
            node(2, Some(1), "blocked-dir", 700, NodeKind::Directory),
            node(3, Some(2), "protected-file", 50, NodeKind::File),
            node(4, Some(1), "eligible-dir", 300, NodeKind::Directory),
            node(5, Some(1), "unrelated-file", 20, NodeKind::File),
        ],
        evidence: vec![
            evidence(2, EvidenceRelation::Rebuildable),
            evidence(3, EvidenceRelation::Protected),
            evidence(4, EvidenceRelation::Rebuildable),
        ],
    }
}

#[test]
fn positive_target_candidates_skip_unrelated_corrupt_nodes_and_report_the_gap() {
    let fixture = tempfile::tempdir().unwrap();
    let database = fixture.path().join("graph.sqlite");
    let mut store = SqliteSnapshotStore::open(&database).unwrap();
    store.save(&graph()).unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE nodes SET name = X'ff' WHERE snapshot_id = 'candidate-snapshot' AND id = 5",
            [],
        )
        .unwrap();
    assert!(store.load("candidate-snapshot").is_err());

    let result = store
        .candidate_selection("candidate-snapshot", 500, QueryBudget::default())
        .unwrap();
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].0.id, 4);
    assert_eq!(result.selected_bytes, 300);
    assert_eq!(result.remaining_bytes, 200);
    assert!(result.complete);
    assert_eq!(result.truncated, None);
}

#[test]
fn candidate_result_never_calls_a_budget_cut_complete() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = graph();
    graph
        .evidence
        .retain(|edge| edge.relation != EvidenceRelation::Protected);
    store.save(&graph).unwrap();
    let budget = QueryBudget {
        max_nodes: 1,
        ..QueryBudget::default()
    };
    let result = store
        .candidate_selection("candidate-snapshot", 900, budget)
        .unwrap();
    assert_eq!(result.candidates.len(), 1);
    assert!(!result.complete);
    assert_eq!(result.truncated, Some(TruncationReason::NodeLimit));
    assert!(result.remaining_bytes > 0);
}

#[test]
fn migrated_prestructured_directory_remains_a_candidate() {
    let fixture = tempfile::tempdir().unwrap();
    let database = fixture.path().join("legacy.sqlite");
    let mut store = SqliteSnapshotStore::open(&database).unwrap();
    let graph = graph();
    store.save(&graph).unwrap();
    let legacy = serde_json::to_string(&graph.nodes[3]).unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE nodes SET kind = NULL, node_json = ?1 WHERE snapshot_id = 'candidate-snapshot' AND id = 4",
            [legacy],
        )
        .unwrap();
    let result = store
        .candidate_selection("candidate-snapshot", 1, QueryBudget::default())
        .unwrap();
    assert_eq!(result.candidates[0].0.id, 4);
}

/// 核对合法夹具的候选、保护传播及目标缺口；来源：原生 Rust Store 公共候选 API。
fn assert_protected_candidate_and_gap(result: &CandidateSelection) {
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].0.id, 4);
    assert!(
        result.candidates[0]
            .1
            .iter()
            .any(|edge| edge.relation == EvidenceRelation::Rebuildable)
    );
    // protected-file 的保护必须继续排除其 blocked-dir 祖先，不能因头准入而丢失。
    assert!(
        !result
            .candidates
            .iter()
            .any(|(node, _)| node.id == 2 || node.id == 3)
    );
    assert_eq!(result.selected_bytes, 300);
    assert_eq!(result.remaining_bytes, 200);
    assert!(result.coverage_complete);
    assert!(result.complete);
    assert_eq!(result.truncated, None);
}

/// 合法大 volume 标识通过 save 持久化，初始头超限必须明确拒绝；来源：Q09 候选准备原始准入。
#[test]
fn oversized_legal_snapshot_header_is_rejected_by_candidate_raw_budget() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut fixture = graph();
    fixture.snapshot.volume_id = Some("v".repeat(2 * 1024 * 1024));
    store.save(&fixture).unwrap();
    // 即使目标为零也须先确认覆盖头，不能把超预算伪造成空完整结果。
    for target in [500, 0] {
        let result =
            store.candidate_selection(&fixture.snapshot.id, target, QueryBudget::default());
        assert!(
            matches!(result, Err(StoreError::BudgetExceeded)),
            "target={target}: legal oversized header must fail raw admission"
        );
    }
}

/// 普通头和足额大头保留同一候选语义；来源：Q09 合法头准入正控制。
#[test]
fn admitted_snapshot_headers_preserve_candidate_protection_gap_and_completeness() {
    for large in [false, true] {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let mut fixture = graph();
        let budget = if large {
            fixture.snapshot.volume_id = Some("v".repeat(2 * 1024 * 1024));
            QueryBudget {
                max_response_bytes: 4 * 1024 * 1024,
                ..QueryBudget::default()
            }
        } else {
            QueryBudget::default()
        };
        store.save(&fixture).unwrap();
        let result = store
            .candidate_selection(&fixture.snapshot.id, 500, budget)
            .unwrap();
        assert_protected_candidate_and_gap(&result);
    }
}

/// 头可单独准入不代表节点/必需证据有新额度；来源：Q09 同一原始字段账本。
#[test]
fn candidate_snapshot_header_and_required_evidence_share_remaining_raw_bytes() {
    let mut fixture = graph();
    fixture
        .evidence
        .iter_mut()
        .find(|edge| edge.node_id == 4)
        .unwrap()
        .subject = "e".repeat(4096);
    // 同一合法节点及证据在普通头下完整成功，排除响应本身过大或保护规则影响。
    let mut ordinary = SqliteSnapshotStore::open_in_memory().unwrap();
    ordinary.save(&fixture).unwrap();
    assert_protected_candidate_and_gap(
        &ordinary
            .candidate_selection(&fixture.snapshot.id, 500, QueryBudget::default())
            .unwrap(),
    );

    let budget = QueryBudget::default();
    fixture.snapshot.volume_id = Some(String::new());
    let fixed_header_bytes = serde_json::to_vec(&fixture.snapshot).unwrap().len();
    fixture.snapshot.volume_id =
        Some("v".repeat(budget.max_response_bytes - fixed_header_bytes - 1024));
    assert_eq!(
        serde_json::to_vec(&fixture.snapshot).unwrap().len(),
        budget.max_response_bytes - 1024
    );
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&fixture).unwrap();

    // 公共有界快照读取证明头本身可准入且只余 1 KiB；没有损坏 SQL 或隐藏夹具写入。
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(budget.deadline_ms);
    let mut header_reads = diskgraph_core::QueryReadBudget::new(budget, deadline).unwrap();
    store
        .snapshot_with_budget(&fixture.snapshot.id, &mut header_reads)
        .unwrap();
    assert_eq!(header_reads.remaining_raw_bytes(), 1024);

    let result = store
        .candidate_selection(&fixture.snapshot.id, 500, budget)
        .unwrap();
    assert!(
        result.candidates.is_empty(),
        "header consumption must not reset before required evidence"
    );
    assert_eq!(result.selected_bytes, 0);
    assert_eq!(result.remaining_bytes, 500);
    assert!(result.coverage_complete);
    assert!(!result.complete);
    assert_eq!(result.truncated, Some(TruncationReason::ByteLimit));

    let enough = QueryBudget {
        max_response_bytes: budget.max_response_bytes + 8192,
        ..budget
    };
    assert_protected_candidate_and_gap(
        &store
            .candidate_selection(&fixture.snapshot.id, 500, enough)
            .unwrap(),
    );
}

/// 已过期准备保留空 Deadline，而非伪造已读覆盖；来源：Q09 公共 Store 候选期限契约。
fn assert_expired_header_is_explicitly_unobserved(large: bool) {
    let mut fixture = graph();
    if large {
        fixture.snapshot.volume_id = Some("v".repeat(2 * 1024 * 1024));
    }
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&fixture).unwrap();
    for target in [0, 500] {
        let deadline = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_millis(1))
            .unwrap();
        let result = store
            .candidate_selection_until(
                &fixture.snapshot.id,
                target,
                QueryBudget::default(),
                deadline,
            )
            .expect("expired preparation retains a typed Deadline result");
        assert!(result.candidates.is_empty());
        assert_eq!(result.selected_bytes, 0);
        assert_eq!(result.remaining_bytes, target);
        assert!(!result.complete);
        assert_eq!(result.truncated, Some(TruncationReason::Deadline));
        assert!(!result.coverage_complete);
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["coverage_observed"], serde_json::Value::Bool(false));
        assert_eq!(wire["coverage_complete"], serde_json::Value::Bool(false));
    }
}

/// 普通合法头也必须遵守先期限后读取；来源：Q09 已过期公开请求回归。
#[test]
fn expired_candidate_deadline_keeps_an_unobserved_ordinary_header() {
    assert_expired_header_is_explicitly_unobserved(false);
}

/// 过期优先于巨型合法头解码/字节准入，不能变成 BudgetExceeded；来源：Q09 准备阶段期限。
#[test]
fn expired_candidate_deadline_keeps_an_unobserved_oversized_header() {
    assert_expired_header_is_explicitly_unobserved(true);
}

/// 真正读取完整头后显式标记已观察，不改变候选保护语义；来源：Q09 覆盖诊断正控制。
#[test]
fn admitted_complete_candidate_header_is_explicitly_observed() {
    let fixture = graph();
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&fixture).unwrap();
    let result = store
        .candidate_selection(&fixture.snapshot.id, 500, QueryBudget::default())
        .unwrap();
    assert_protected_candidate_and_gap(&result);
    let wire = serde_json::to_value(&result).unwrap();
    assert_eq!(wire["coverage_observed"], serde_json::Value::Bool(true));
    assert_eq!(wire["coverage_complete"], serde_json::Value::Bool(true));
}

/// 合法部分覆盖与“尚未观察”必须可区分；来源：Q09 公共 save/query 的真实缺口回归。
#[test]
fn admitted_partial_candidate_header_is_observed_with_real_coverage_gap() {
    let mut fixture = graph();
    fixture.snapshot.coverage.complete = false;
    fixture.snapshot.coverage.unreadable_nodes = 1;
    let unreadable = fixture.nodes.iter_mut().find(|node| node.id == 5).unwrap();
    unreadable.read_error = true;
    unreadable.size_known = false;
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&fixture).unwrap();
    let result = store
        .candidate_selection(&fixture.snapshot.id, 500, QueryBudget::default())
        .unwrap();
    assert!(result.candidates.is_empty());
    assert_eq!(result.selected_bytes, 0);
    assert_eq!(result.remaining_bytes, 500);
    assert!(!result.complete);
    assert!(!result.coverage_complete);
    assert_eq!(result.truncated, None);
    let wire = serde_json::to_value(&result).unwrap();
    assert_eq!(wire["coverage_observed"], serde_json::Value::Bool(true));
    assert_eq!(wire["coverage_complete"], serde_json::Value::Bool(false));
}
