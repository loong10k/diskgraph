//! 隔离合成元数据经真实发布/CLI dispatch 验证；不代替平台扫描验收。
use crate::cli::Cli;
use crate::dispatch::dispatch;
use clap::Parser;
use diskgraph_core::{
    Authorizer, BusinessError, Decision, DiskGraph, DiskNode, DiskSnapshot, NodeKind, Permission,
    PrincipalId, ResourceLocator, ScanCoverage, ScanSettings, ScopeId,
};
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use diskgraph_store::{ControlStore, SqliteSnapshotStore};
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

pub(super) fn fixture() -> (tempfile::TempDir, Engine, PrincipalId, ScopeId) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("private_root");
    std::fs::create_dir(&root).unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = PrincipalId::new("local-user").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let locator = ResourceLocator::NativePath(root.to_string_lossy().into_owned());
    let node = DiskNode {
        id: 1,
        parent_id: None,
        locator: locator.clone(),
        name: "private_root".into(),
        kind: NodeKind::Directory,
        subtree_bytes: 100,
        size_known: true,
        direct_bytes: 0,
        files: 1,
        directories: 1,
        modified_unix_seconds: None,
        file_identity: None,
        category_hint: None,
        reclaim_hint: None,
        read_error: false,
    };
    let mut child = node.clone();
    child.id = 2;
    child.parent_id = Some(1);
    child.name = "秘密Ä".into();
    child.locator = ResourceLocator::NativePath(root.join("秘密Ä").to_string_lossy().into_owned());
    let graph = DiskGraph {
        snapshot: DiskSnapshot {
            id: "fixture".into(),
            root: locator,
            volume_id: None,
            captured_at_unix_ms: 1,
            settings: ScanSettings {
                apparent_size: false,
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
        nodes: vec![node, child],
        evidence: vec![],
    };
    let mut writer =
        SqliteSnapshotStore::open(&directory.path().join("data/diskgraph.sqlite")).unwrap();
    writer
        .append_staging_nodes("fixture", &graph.nodes)
        .unwrap();
    writer
        .publish_revision_owned(
            "fixture",
            &graph,
            "fixture-revision",
            1,
            Some((engine.server_id().unwrap().as_str(), scope.as_str())),
        )
        .unwrap();
    (directory, engine, actor, scope)
}

fn cli(command: &str, scope: &ScopeId) -> Cli {
    let mut args = vec!["diskgraph", "--json", command, "--scope", scope.as_str()];
    if command == "search" {
        args.extend(["--pattern", "秘密ä"]);
    }
    Cli::parse_from(args)
}

/// 原能力快照允许，但第二次能力观察撤销真实控制库scope。
struct Revoker {
    policy: diskgraph_core::PolicyAuthorizer,
    control: RefCell<ControlStore>,
    calls: Cell<usize>,
}
impl Authorizer for Revoker {
    fn decide(&self, actor: &PrincipalId, permission: &Permission, scope: &ScopeId) -> Decision {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == 2 {
            self.control.borrow_mut().revoke_scope(scope).unwrap();
        }
        self.policy.decide(actor, permission, scope)
    }
    fn policy_version(&self) -> u64 {
        self.policy.policy_version()
    }
}

#[test]
fn actual_narrow_commands_refuse_expired_original_dispatch_deadline() {
    for command in ["node", "children", "top", "explore", "search"] {
        let (_directory, engine, actor, scope) = fixture();
        let mut output = vec![];
        let result = dispatch(
            &engine,
            &cli(command, &scope),
            &actor,
            &engine.policy_authorizer().unwrap(),
            &mut output,
            Instant::now(),
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::BudgetExceeded))
            ),
            "{command} escaped expired deadline: {result:?} {output:?}"
        );
        assert!(output.is_empty());
    }
}

#[test]
fn actual_narrow_commands_observe_terminal_revocation_before_output() {
    for command in ["node", "children", "top", "explore", "search"] {
        let (directory, engine, actor, scope) = fixture();
        let policy = Revoker {
            policy: engine.policy_authorizer().unwrap(),
            control: RefCell::new(
                ControlStore::open(&directory.path().join("data/diskgraph-control.sqlite"))
                    .unwrap(),
            ),
            calls: Cell::new(0),
        };
        let mut output = vec![];
        let result = dispatch(
            &engine,
            &cli(command, &scope),
            &actor,
            &policy,
            &mut output,
            Instant::now() + Duration::from_secs(5),
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ),
            "{command} escaped terminal revocation: {result:?} {output:?}"
        );
        assert!(output.is_empty());
    }
}

#[test]
fn actual_narrow_commands_preserve_allowed_wire_fields_and_unicode() {
    let (_directory, engine, actor, scope) = fixture();
    for command in ["node", "children", "top", "explore", "search"] {
        let mut output = vec![];
        dispatch(
            &engine,
            &cli(command, &scope),
            &actor,
            &engine.policy_authorizer().unwrap(),
            &mut output,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&output[0]).unwrap();
        assert_eq!(value["data"]["revision_id"], "fixture-revision");
        if command == "search" {
            assert_eq!(value["data"]["items"][0]["name"], "秘密Ä");
        }
    }
}

#[test]
fn actual_narrow_commands_deny_ungranted_principal() {
    let (_directory, engine, _owner, scope) = fixture();
    let actor = PrincipalId::new("ungranted").unwrap();
    for command in ["node", "children", "top", "explore", "search"] {
        let mut output = vec![];
        let result = dispatch(
            &engine,
            &cli(command, &scope),
            &actor,
            &engine.policy_authorizer().unwrap(),
            &mut output,
            Instant::now() + Duration::from_secs(5),
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ),
            "{command}: {result:?}"
        );
        assert!(output.is_empty());
    }
}

/// 第二次能力检查明确允许，但耗时超过固定50ms窗口。
struct LatePolicy {
    policy: diskgraph_core::PolicyAuthorizer,
    calls: Cell<usize>,
}
impl Authorizer for LatePolicy {
    fn decide(&self, actor: &PrincipalId, permission: &Permission, scope: &ScopeId) -> Decision {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == 2 {
            std::thread::sleep(Duration::from_millis(60));
        }
        self.policy.decide(actor, permission, scope)
    }
    fn policy_version(&self) -> u64 {
        self.policy.policy_version()
    }
}

#[test]
fn actual_narrow_commands_refuse_late_terminal_capability() {
    let (_directory, engine, actor, scope) = fixture();
    for command in ["node", "children", "top", "explore", "search"] {
        let policy = LatePolicy {
            policy: engine.policy_authorizer().unwrap(),
            calls: Cell::new(0),
        };
        let mut output = vec![];
        let result = dispatch(
            &engine,
            &cli(command, &scope),
            &actor,
            &policy,
            &mut output,
            Instant::now() + Duration::from_secs(5),
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::BudgetExceeded))
            ),
            "{command}: {result:?}"
        );
        assert!(output.is_empty());
    }
}

#[test]
fn scope_bound_reader_refuses_foreign_scope_before_consumer() {
    let (directory, engine, actor, _scope) = fixture();
    let root = directory.path().join("other_root");
    std::fs::create_dir(&root).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let other = engine.register_scope(&root, &actor, &policy).unwrap();
    let called = Cell::new(false);
    let result = engine.with_authorized_revision_reader_for_scope_until(
        "fixture-revision",
        &other,
        &actor,
        &policy,
        Instant::now() + Duration::from_secs(5),
        |_, _, _| {
            called.set(true);
            Ok(())
        },
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    assert!(!called.get());
}

#[test]
fn search_cursor_uses_dispatch_identity_instead_of_cli_label() {
    let (_directory, engine, actor, scope) = fixture();
    let policy = engine.policy_authorizer().unwrap();
    let context = diskgraph_core::CursorContext {
        principal_binding: "spoofed-label",
        scope_id: scope.as_str(),
        revision_id: "fixture-revision",
        filter_binding: "pattern:",
        sort_binding: "name_asc,id_asc,keyset_v2",
        policy_version: policy.policy_version(),
    };
    let encoded = diskgraph_core::SearchCursor {
        version: 2,
        binding: diskgraph_core::PagingCursor::issue(&context, "pattern:", context.sort_binding, 1),
        last_name: "private_root".into(),
        last_id: 1,
    }
    .encode();
    let cli = Cli::parse_from([
        "diskgraph",
        "--json",
        "--principal",
        "spoofed-label",
        "search",
        "--scope",
        scope.as_str(),
        "--pattern",
        "",
        "--cursor",
        &encoded,
    ]);
    let mut output = vec![];
    let result = dispatch(
        &engine,
        &cli,
        &actor,
        &policy,
        &mut output,
        Instant::now() + Duration::from_secs(5),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::InvalidArgument))
    ));
    assert!(output.is_empty());
}
