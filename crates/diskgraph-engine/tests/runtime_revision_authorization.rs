//! 运行期真实注册隔离与首末读取授权；所有数据库均为隔离夹具。
#![cfg(any(unix, windows))]
use diskgraph_core::{Locator, ResourceLocator};
use diskgraph_engine::{Engine, EngineConfig};
use diskgraph_store::SqliteSnapshotStore;

#[path = "support/legacy_graph.rs"]
mod legacy_graph;
use legacy_graph::legacy_graph;

#[test]
fn in_flight_complete_and_truncated_reads_refuse_runtime_revision_quarantine() {
    use diskgraph_core::{
        Authorizer, BusinessError, Decision, Permission, PolicyAuthorizer, PrincipalId,
        QueryBudget, ScopeId,
    };
    use diskgraph_engine::{EngineError, RevisionDisplayCompletion};
    use std::cell::Cell;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
    #[cfg(windows)]
    use std::os::windows::ffi::OsStringExt;
    /// 在真实能力回调中通过第二 Engine 完成实际注册；来源：原生 Rust SC-04 回归。
    struct RegisterDuringDecision<'a> {
        engine: &'a Engine,
        root: &'a std::path::Path,
        actor: &'a PrincipalId,
        policy: &'a PolicyAuthorizer,
        calls: Cell<usize>,
        trigger: usize,
    }
    impl Authorizer for RegisterDuringDecision<'_> {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &ScopeId,
        ) -> Decision {
            let call = self.calls.get() + 1;
            self.calls.set(call);
            if call == self.trigger {
                self.engine
                    .register_scope(self.root, self.actor, self.policy)
                    .unwrap();
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    let mut incorrectly_allowed = Vec::new();
    for mode in 0..25 {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("r�");
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        #[cfg(unix)]
        let alias_name = std::ffi::OsString::from_vec(b"r\xff".to_vec());
        #[cfg(windows)]
        let alias_name = std::ffi::OsString::from_wide(&[u16::from(b'r'), 0xd800]);
        let alias = root.parent().unwrap().join(alias_name);
        let data = directory.path().join("data");
        let engine = Engine::open(EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        })
        .unwrap();
        let actor = PrincipalId::new("in-flight-legacy-reader").unwrap();
        engine.bootstrap_local_admin(&actor).unwrap();
        // 独立 Engine 先打开；之后导入才建立旧归属，避免启动隔离提前拒绝初次读取。
        let other_engine = Engine::open(EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        })
        .unwrap();
        let healthy = if mode >= 9 {
            let path = directory.path().join("healthy");
            std::fs::create_dir(&path).unwrap();
            let path = path.canonicalize().unwrap();
            let scope = engine
                .register_scope(&path, &actor, &engine.policy_authorizer().unwrap())
                .unwrap();
            Some((path, scope))
        } else {
            None
        };
        // 可信历史导入模拟旧绑定；实际终检触发器仍为正常 Engine 注册，不伪造拒绝行。
        let (server, scope) = {
            let mut control = engine.control_store().unwrap();
            let server = control.ensure_server().unwrap();
            let scope = control
                .register_scope(&Locator::from_native_path(&alias), None)
                .unwrap();
            let version = control.policy_version().unwrap();
            control
                .upsert_grant(&diskgraph_core::Grant {
                    principal: actor.clone(),
                    permission: diskgraph_core::Permission::MetadataRead,
                    scope: scope.clone(),
                    policy_version: version,
                })
                .unwrap();
            (server, scope)
        };
        let graph_path = data.join("diskgraph.sqlite");
        SqliteSnapshotStore::open(&graph_path)
            .unwrap()
            .publish_revision(
                "legacy-job",
                &legacy_graph(ResourceLocator::NativePath(root.to_str().unwrap().into())),
                "legacy-revision",
                1,
            )
            .unwrap();
        rusqlite::Connection::open(&graph_path)
            .unwrap()
            .execute(
                "INSERT INTO revision_ownership(revision_id,server_id,scope_id) VALUES (?1,?2,?3)",
                rusqlite::params!["legacy-revision", server.as_str(), scope.as_str()],
            )
            .unwrap();
        if let Some((healthy_root, healthy_scope)) = healthy {
            let mut graph = legacy_graph(ResourceLocator::NativePath(
                healthy_root.to_str().unwrap().into(),
            ));
            graph.snapshot.id = "healthy-snapshot".into();
            SqliteSnapshotStore::open(&graph_path)
                .unwrap()
                .publish_revision("healthy-job", &graph, "healthy-revision", 2)
                .unwrap();
            rusqlite::Connection::open(&graph_path).unwrap().execute(
                "INSERT INTO revision_ownership(revision_id,server_id,scope_id) VALUES (?1,?2,?3)",
                rusqlite::params!["healthy-revision", server.as_str(), healthy_scope.as_str()],
            ).unwrap();
        }
        let policy = engine.policy_authorizer().unwrap();
        let callback = RegisterDuringDecision {
            engine: &other_engine,
            root: &root,
            actor: &actor,
            policy: &policy,
            calls: Cell::new(0),
            trigger: if mode >= 17 {
                6
            } else if mode >= 9 {
                4
            } else if mode == 4 {
                1
            } else if mode == 6 {
                3
            } else {
                2
            },
        };
        let isolate = |reader: &SqliteSnapshotStore| {
            assert!(
                reader
                    .revision_ownership("legacy-revision")
                    .unwrap()
                    .is_some()
            );
            engine.register_scope(&root, &actor, &policy).unwrap();
            assert!(
                engine
                    .authorize_revision(None, "legacy-revision", &actor, &policy)
                    .is_err()
            );
        };
        let result = if mode == 0 {
            engine.with_authorized_revision_reader(
                "legacy-revision",
                &actor,
                &policy,
                1000,
                |reader, snapshot, _| {
                    assert_eq!(snapshot, "legacy-snapshot");
                    isolate(reader);
                    Ok(())
                },
            )
        } else if mode < 3 {
            engine.with_authorized_revision_display_reader_bounded(
                "legacy-revision",
                &actor,
                &policy,
                QueryBudget {
                    deadline_ms: 1000,
                    ..QueryBudget::default()
                },
                |reader, snapshot, _| {
                    assert_eq!(snapshot, "legacy-snapshot");
                    isolate(reader);
                    Ok(if mode == 1 {
                        RevisionDisplayCompletion::Complete
                    } else {
                        RevisionDisplayCompletion::Truncated
                    })
                },
            )
        } else {
            // 普通候选真实成功作为对照，不能把原有准备失败误记为安全拒权。
            assert!(
                engine
                    .review_candidates(
                        "legacy-revision",
                        0,
                        QueryBudget::default(),
                        &actor,
                        &policy
                    )
                    .is_ok()
            );
            let result = match mode {
                3 | 6 => engine
                    .review_candidates(
                        "legacy-revision",
                        0,
                        QueryBudget::default(),
                        &actor,
                        &callback,
                    )
                    .map(|_| ()),
                4 => engine
                    .finalize_revision_read_until(
                        "legacy-revision",
                        &actor,
                        &callback,
                        std::time::Instant::now() + std::time::Duration::from_secs(1),
                    )
                    .map(|_| ()),
                7 => engine.with_authorized_revision_reader(
                    "legacy-revision",
                    &actor,
                    &callback,
                    1000,
                    |_, _, _| Ok(()),
                ),
                5 | 8 => engine.with_authorized_revision_display_reader_bounded(
                    "legacy-revision",
                    &actor,
                    &callback,
                    QueryBudget {
                        deadline_ms: 1000,
                        ..QueryBudget::default()
                    },
                    |_, _, _| {
                        Ok(if mode == 8 {
                            RevisionDisplayCompletion::Truncated
                        } else {
                            RevisionDisplayCompletion::Complete
                        })
                    },
                ),
                9..=24 => {
                    let index = (mode - 9) % 8;
                    let (left, right) = if index % 2 == 0 {
                        ("legacy-revision", "healthy-revision")
                    } else {
                        ("healthy-revision", "legacy-revision")
                    };
                    let history = |auth: &dyn Authorizer| {
                        let budget = QueryBudget::default();
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(1);
                        match index / 2 {
                            0 => engine
                                .compare_revisions_until(
                                    left, right, 0, budget, &actor, auth, deadline,
                                )
                                .map(|_| ()),
                            1 => engine
                                .revision_changes_until(left, right, budget, &actor, auth, deadline)
                                .map(|_| ()),
                            2 => engine
                                .growth_between_until(
                                    left,
                                    right,
                                    std::path::Path::new(""),
                                    budget,
                                    &actor,
                                    auth,
                                    deadline,
                                )
                                .map(|_| ()),
                            3 => engine
                                .sync_plan_until(
                                    left,
                                    right,
                                    diskgraph_core::SyncMethod::Mirror,
                                    0,
                                    budget,
                                    &actor,
                                    auth,
                                    deadline,
                                )
                                .map(|_| ()),
                            _ => unreachable!("history fixture family"),
                        }
                    };
                    let ordinary = history(&policy);
                    assert!(
                        ordinary.is_ok(),
                        "mode {mode} healthy history control failed: {ordinary:?}"
                    );
                    history(&callback)
                }
                _ => unreachable!("test fixture mode"),
            };
            assert!(
                callback.calls.get() >= callback.trigger,
                "mode {mode} missed registration callback"
            );
            result
        };
        if !matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ) {
            incorrectly_allowed.push((mode, format!("{result:?}")));
        }
        assert!(
            SqliteSnapshotStore::open(&graph_path)
                .unwrap()
                .revision_ownership_for_audit("legacy-revision")
                .unwrap()
                .is_some()
        );
    }
    assert!(
        incorrectly_allowed.is_empty(),
        "quarantined requests returned: {incorrectly_allowed:?}"
    );
}
