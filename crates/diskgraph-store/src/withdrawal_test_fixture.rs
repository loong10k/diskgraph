//! D45 真实控制库夹具：使用公开注册和授权接口，保持 SQLite FULL。
use crate::ControlStore;
use diskgraph_core::{Grant, Locator, Permission, PrincipalId, ScopeId};
use std::path::PathBuf;

/// 独占隔离控制库与已持久授权；来源：原生 Rust ControlStore 的公开建库和策略接口。
pub(super) struct WithdrawalFixture {
    pub(super) store: ControlStore,
    pub(super) scope: ScopeId,
    pub(super) principal: PrincipalId,
    pub(super) path: PathBuf,
    pub(super) directory: tempfile::TempDir,
}

impl WithdrawalFixture {
    /// 建立真实 FULL 数据库及 epoch 1 的 MetadataRead 授权。
    /// 参数：无；返回：仅持有本测试临时目录的夹具。
    pub(super) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("control.sqlite");
        let mut store = ControlStore::open(&path).unwrap();
        store.ensure_server().unwrap();
        let scope = store
            .register_scope(&Locator::from_native_path(directory.path()), None)
            .unwrap();
        let principal = PrincipalId::new("withdrawal-reader").unwrap();
        store.publish_policy_version(1).unwrap();
        store
            .upsert_grant(&Grant {
                principal: principal.clone(),
                permission: Permission::MetadataRead,
                scope: scope.clone(),
                policy_version: 1,
            })
            .unwrap();
        let full: i64 = store
            .connection
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        assert_eq!(full, 2, "real FULL persistence is a prerequisite");
        assert_eq!(
            store
                .live_permission(&principal, &Permission::MetadataRead, &scope)
                .unwrap(),
            Some(true)
        );
        Self {
            store,
            scope,
            principal,
            path,
            directory,
        }
    }

    /// 打开同一实际数据库的第二个独立 SQLite 连接。
    /// 参数：无；返回：公开 open 创建的独立 ControlStore。
    pub(super) fn second(&self) -> ControlStore {
        ControlStore::open(&self.path).unwrap()
    }

    /// 通过 SQLite 官方 backup 生成具有相同 server 与数据的不同文件。
    /// 参数：无；返回：备份文件上的独立 ControlStore。
    pub(super) fn backup(&self) -> ControlStore {
        let path = self.directory.path().join("backup.sqlite");
        self.store
            .connection
            .backup(rusqlite::MAIN_DB, &path, None)
            .unwrap();
        ControlStore::open(&path).unwrap()
    }

    /// 重新持久授予当前 epoch 的原权限。
    /// 参数：无；返回：授予失败时测试失败。
    pub(super) fn regrant(&mut self) {
        self.store
            .upsert_grant(&Grant {
                principal: self.principal.clone(),
                permission: Permission::MetadataRead,
                scope: self.scope.clone(),
                policy_version: 1,
            })
            .unwrap();
    }
}
