//! 合法大范围字段与真实索引夹具；来源：D42 EC-04 公开 Control/Engine API。
use diskgraph_core::{JobRequestAuthority, Locator, Permission, PrincipalId, ScopeId};
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use diskgraph_store::{JobRecord, JobState};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// 真实可寻址文件、不可变范围与原生索引；来源：Rust Process 入队整窗口验收。
/// 显示字段在首次公开注册时写入，不修改既存 scope，不伪造 epoch 或扫描记录。
pub(crate) struct Fixture {
    _source: tempfile::TempDir,
    _data: tempfile::TempDir,
    pub(crate) engine: Engine,
    pub(crate) actor: PrincipalId,
    pub(crate) scope: ScopeId,
    pub(crate) base: String,
    pub(crate) node: u64,
    pub(crate) field_bytes: usize,
}
impl Fixture {
    /// 参数：首次注册显示名和卷标各自的字节数，零表示普通原生显示名；返回：已真实索引的隔离夹具。
    pub(crate) fn new(field_bytes: usize) -> Self {
        #[cfg(target_os = "linux")]
        let source =
            tempfile::tempdir_in("/dev/shm").expect("requires real Linux tmpfs epoch support");
        #[cfg(not(target_os = "linux"))]
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("target"), b"metadata-only entry fixture").unwrap();
        let root = source.path().canonicalize().unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: data.path().to_owned(),
            ..EngineConfig::default()
        })
        .unwrap();
        let actor = PrincipalId::new("entry-scope-budget").unwrap();
        engine.bootstrap_local_admin(&actor).unwrap();
        let mut locator = Locator::from_native_path(&root);
        if field_bytes != 0 {
            locator.display = "d".repeat(field_bytes);
        }
        let volume = "v".repeat(field_bytes);
        let scope = engine
            .control_store()
            .unwrap()
            .register_scope(&locator, Some(&volume))
            .unwrap();
        // 原 Engine 注册真实根并授予现有权限；幂等 Store 注册保持首次合法元数据。
        assert_eq!(
            engine
                .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
                .unwrap(),
            scope
        );
        let auth = engine.policy_authorizer().unwrap();
        let scan = engine.index_scope(&scope, &actor, &auth).unwrap();
        assert_eq!(
            engine
                .run_job(&scan.job_id, "scope-fixture-index")
                .unwrap()
                .state,
            JobState::Completed
        );
        let base = engine
            .revision_for_job(&scan.job_id, &actor, &auth)
            .unwrap();
        let node = engine
            .revision_node_at(&base, Path::new("target"))
            .unwrap()
            .unwrap()
            .id;
        assert!(engine.queued_jobs().unwrap().is_empty());
        Self {
            _source: source,
            _data: data,
            engine,
            actor,
            scope,
            base,
            node,
            field_bytes,
        }
    }
    /// 参数：原请求能力上限；返回：真实主体、固定绝对 60 秒有效期的请求，不包含 ContentRead。
    pub(crate) fn authority(&self, capabilities: Vec<Permission>) -> JobRequestAuthority {
        JobRequestAuthority::authenticated_remote(
            self.actor.clone(),
            "scope-budget-fixture",
            "http",
            capabilities,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 60,
        )
        .unwrap()
    }
    /// 参数：公开入队实际结果；返回：验证精确平台资格及真实持久状态，无发布。
    pub(crate) fn assert_platform_result(&self, result: Result<JobRecord, EngineError>) {
        #[cfg(target_os = "linux")]
        {
            let job = result.expect("real tmpfs scan must capture a qualified persistent epoch; unsupported is not a pass");
            assert_eq!(job.kind, diskgraph_store::JobKind::ProcessEvidence);
            assert_eq!(job.state, JobState::Queued);
            assert_eq!(job.scope_id, self.scope);
            assert_eq!(job.principal, self.actor);
            let control = self.engine.control_store().unwrap();
            assert_eq!(control.job(&job.job_id).unwrap(), job);
            let input = control.process_evidence_job_input(&job.job_id).unwrap();
            assert_eq!(input.base_revision_id(), self.base);
            assert_eq!(input.node_id(), self.node);
            drop(control);
            assert_eq!(self.engine.queued_jobs().unwrap().len(), 1);
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(
                matches!(
                    result,
                    Err(EngineError::Business(
                        diskgraph_core::BusinessError::Unsupported
                    ))
                ),
                "unqualified native platform must remain Unsupported: {result:?}"
            );
            self.assert_no_queue();
        }
        self.assert_no_publication();
    }
    /// 参数：无；返回：队列确实为空的断言，拒绝不得暗中入队。
    pub(crate) fn assert_no_queue(&self) {
        assert!(self.engine.queued_jobs().unwrap().is_empty());
        self.assert_no_publication();
    }
    /// 参数：无；返回：仍指向原始扫描 revision 的断言。
    pub(crate) fn assert_no_publication(&self) {
        assert_eq!(
            self.engine.latest_revision(&self.scope).unwrap().as_deref(),
            Some(self.base.as_str())
        );
    }
}
