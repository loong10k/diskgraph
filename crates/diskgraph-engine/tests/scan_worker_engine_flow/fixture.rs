use diskgraph_core::{Authorizer, Decision, Permission, PolicyAuthorizer, PrincipalId, ScopeId};
use diskgraph_engine::{
    Engine, EngineConfig, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRecovery,
    ScanWorkerRuntimeBudget, admin_scope,
};
use diskgraph_scan_worker::ProtocolLimits;
use diskgraph_store::JobRecord;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

/// 真实Cargo helper与公开持久任务夹具；来源：PF-06 Engine实际consumer验收。
/// artifact仅由测试宿主显式提供，预期从这份构建独立获取，不用于产品安装自信。
pub(super) struct Fixture {
    pub engine: Engine,
    pub recovery: ScanWorkerRecovery,
    pub scope: ScopeId,
    pub actor: PrincipalId,
    pub policy: PolicyAuthorizer,
    pub root: PathBuf,
    pub directory: tempfile::TempDir,
}

impl Fixture {
    /// 参数：response为该案原响应上限，wrong_hash选独立错误预期，replace_path实改镜像名称。
    /// 返回：真实held原镜像、已bootstrap/register/grant的Engine与外部Recovery。
    pub(super) fn new(response: ProtocolLimits, wrong_hash: bool, replace_path: bool) -> Self {
        let artifact = match std::env::var_os("DISKGRAPH_ENGINE_SCAN_WORKER") {
            Some(path) => PathBuf::from(path),
            None => panic!("root must provide the actual Cargo worker artifact explicitly"),
        };
        assert!(artifact.is_absolute() && artifact.is_file());
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("held-helper");
        std::fs::copy(&artifact, &image).unwrap();
        let mut held = File::open(&image).unwrap();
        let length = held.metadata().unwrap().len();
        assert!(length > 0 && length <= 128 << 20);
        let mut hasher = Sha256::new();
        let mut block = [0; 64 << 10];
        loop {
            let read = held.read(&mut block).unwrap();
            if read == 0 {
                break;
            }
            hasher.update(&block[..read]);
        }
        let mut digest: [u8; 32] = hasher.finalize().into();
        if wrong_hash {
            digest[0] ^= 1;
        }
        let expected = ScanWorkerHostConfig::from_expected_image(digest, length).unwrap();
        let runtime = ScanWorkerRuntimeBudget::new(response, 0, 1).unwrap();
        let host = ScanWorkerHost::new(held, expected, runtime).unwrap();
        if replace_path {
            std::fs::rename(&image, directory.path().join("original-image-moved")).unwrap();
            std::fs::write(&image, b"replacement must not become an execution image").unwrap();
        }
        let root = directory.path().join("scope");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("one"), b"actual helper observed content").unwrap();
        std::fs::create_dir(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/two"), b"second file").unwrap();
        let (engine, recovery) = Engine::open_with_scan_worker(
            EngineConfig {
                data_dir: directory.path().join("data"),
                ..EngineConfig::default()
            },
            host,
        )
        .unwrap();
        let actor = PrincipalId::new("engine-physical-worker-test").unwrap();
        let mut policy = PolicyAuthorizer::new(1);
        policy.grant(actor.clone(), Permission::ScopeAdmin, admin_scope());
        engine.bootstrap_local_admin(&actor).unwrap();
        assert_eq!(
            engine.policy_authorizer().unwrap().decide(
                &actor,
                &Permission::ScopeAdmin,
                &admin_scope()
            ),
            Decision::Allowed
        );
        let scope = engine.register_scope(&root, &actor, &policy).unwrap();
        for permission in [
            Permission::IndexWrite,
            Permission::MetadataRead,
            Permission::OperationView,
        ] {
            policy.grant(actor.clone(), permission, scope.clone());
        }
        assert_eq!(
            engine
                .control_store()
                .unwrap()
                .live_permission(&actor, &Permission::IndexWrite, &scope)
                .unwrap(),
            Some(true)
        );
        Self {
            engine,
            recovery,
            scope,
            actor,
            policy,
            root,
            directory,
        }
    }

    /// 参数：无；返回：实际持久Queued Index，不伪造权限或跳过真实claim。
    pub(super) fn enqueue(&self) -> JobRecord {
        self.engine
            .index_scope(&self.scope, &self.actor, &self.policy)
            .unwrap()
    }
}

/// 参数：无；返回：测试宿主明确的有限响应额度，不借staging默认2GiB。
pub(super) fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 64 << 10,
        max_stream_bytes: 8 << 20,
        max_nodes: 1000,
        max_depth: 32,
    }
}
