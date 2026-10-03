use crate::{Engine, EngineConfig};
use diskgraph_core::{Locator, Permission, PolicyAuthorizer, PrincipalId, ScopeId};
use std::path::PathBuf;

/// D24 普通文件终态夹具；来源：CT-01/02 的取消与实时撤权契约。
pub(super) struct ReadTerminalFixture {
    pub(super) directory: tempfile::TempDir,
    pub(super) engine: Engine,
    pub(super) principal: PrincipalId,
    pub(super) scope: ScopeId,
    pub(super) other: ScopeId,
    pub(super) path: PathBuf,
    pub(super) policy: PolicyAuthorizer,
}

impl ReadTerminalFixture {
    pub(super) fn new(persistent_policy: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        for name in ["root", "other"] {
            std::fs::create_dir(directory.path().join(name)).unwrap();
        }
        let root = directory.path().join("root").canonicalize().unwrap();
        let path = root.join("payload");
        std::fs::write(&path, b"12345678").unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: directory.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap();
        let principal = PrincipalId::new("read-terminal-budget").unwrap();
        let (scope, other, policy) = if persistent_policy {
            engine.bootstrap_local_admin(&principal).unwrap();
            let scope = engine
                .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
                .unwrap();
            let other = engine
                .register_scope(
                    &directory.path().join("other"),
                    &principal,
                    &engine.policy_authorizer().unwrap(),
                )
                .unwrap();
            for target in [&scope, &other] {
                engine.set_content_read(target, &principal, true).unwrap();
            }
            (scope, other, engine.policy_authorizer().unwrap())
        } else {
            let mut control = engine.control_store().unwrap();
            let scope = control
                .register_scope(&Locator::from_native_path(&root), None)
                .unwrap();
            let other = control
                .register_scope(
                    &Locator::from_native_path(
                        &directory.path().join("other").canonicalize().unwrap(),
                    ),
                    None,
                )
                .unwrap();
            let mut policy = PolicyAuthorizer::new(1);
            policy.grant(principal.clone(), Permission::ContentRead, scope.clone());
            (scope, other, policy)
        };
        Self {
            directory,
            engine,
            principal,
            scope,
            other,
            path,
            policy,
        }
    }
}
