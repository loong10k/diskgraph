//! D44 文件控制库夹具；原生身份为明确离线协议样本，不宣称 OS 身份资格或源采样验收。
use crate::ControlStore;
use diskgraph_core::{
    Grant, IndexedFileEpoch, JobRequestAuthority, Locator, Permission, PrincipalId,
    ProcessEvidenceJobInput, ProcessEvidenceLimits, ProcessObservationMethod,
};
use rusqlite::Connection;
use rusqlite::types::Value;
use std::path::PathBuf;
use std::time::Duration;

/// 所有持久表的逐列原始值快照，含存在时的序列；来源：D44 隔离存储验收。
pub(super) type PersistedState = Vec<(String, Vec<Vec<Value>>)>;

/// 唯一隔离文件控制库与真实注册/授权；来源：原生 Rust D44 存储入队验收。
pub(super) struct ProcessEnqueueFixture {
    directory: tempfile::TempDir,
    pub(super) store: ControlStore,
    pub(super) input: ProcessEvidenceJobInput,
    pub(super) authority: JobRequestAuthority,
}
impl ProcessEnqueueFixture {
    /// 参数：无；返回：真实注册范围与当前两项 grant 的隔离文件库夹具。
    pub(super) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut store = ControlStore::open(&directory.path().join("control.sqlite")).unwrap();
        let server = store.ensure_server().unwrap();
        let scope = store
            .register_scope(&Locator::from_native_path(directory.path()), None)
            .unwrap();
        let principal = PrincipalId::new("process-enqueue-deadline").unwrap();
        store.publish_policy_version(1).unwrap();
        for permission in [Permission::MetadataRead, Permission::IndexWrite] {
            store
                .upsert_grant(&Grant {
                    principal: principal.clone(),
                    permission,
                    scope: scope.clone(),
                    policy_version: 1,
                })
                .unwrap();
        }
        let input = ProcessEvidenceJobInput::new(
            server,
            scope,
            "deadline-base".into(),
            2,
            ProcessObservationMethod::LinuxProcfsV1,
            IndexedFileEpoch::LinuxHandle {
                device: 1,
                inode: 2,
                filesystem_domain_sha256: [3; 32],
                handle_type: 1,
                handle_bytes: vec![4; 12],
            },
            ProcessEvidenceLimits::default(),
        )
        .unwrap();
        let authority = JobRequestAuthority::authenticated_remote(
            principal,
            "deadline-test-issuer",
            "http",
            vec![Permission::MetadataRead, Permission::IndexWrite],
            ControlStore::now_ms() / 1000 + 60,
        )
        .unwrap();
        Self {
            directory,
            store,
            input,
            authority,
        }
    }

    /// 参数：无；返回：由夹具生命周期持有的控制库文件路径。
    pub(super) fn database_path(&self) -> PathBuf {
        self.directory.path().join("control.sqlite")
    }

    /// 参数：无；返回：重开文件后所有持久表与序列的完整行快照。
    pub(super) fn persisted_state(&self) -> PersistedState {
        let connection = Connection::open(self.database_path()).unwrap();
        let names: Vec<String> = connection
            .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        names
            .into_iter()
            .map(|name| {
                let sql = format!(
                    "SELECT * FROM \"{}\" ORDER BY rowid",
                    name.replace('"', "\"\"")
                );
                let mut statement = connection.prepare(&sql).unwrap();
                let columns = statement.column_count();
                let rows = statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|index| row.get::<_, Value>(index))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                (name, rows)
            })
            .collect()
    }

    /// 参数：宿主毫秒配置；返回：真实连接设置完成，错误使夹具失败。
    pub(super) fn set_busy_timeout(&self, millis: u64) {
        self.store
            .connection
            .busy_timeout(Duration::from_millis(millis))
            .unwrap();
    }

    /// 参数：原宿主毫秒值；返回：断言原配置/事务/真实 VM 查询均已恢复。
    pub(super) fn assert_connection_restored(&self, millis: i64) {
        let timeout: i64 = self
            .store
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(timeout, millis, "actual host busy_timeout changed");
        assert!(
            self.store.connection.is_autocommit(),
            "transaction left open"
        );
        // 调用方先等原期限过去；若过期 progress handler 残留，该真实 VM 查询会中断。
        let total: i64 = self.store.connection.query_row(
            "WITH RECURSIVE n(v) AS (SELECT 0 UNION ALL SELECT v+1 FROM n WHERE v<1024) SELECT sum(v) FROM n",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(total, 524800);
    }
}
