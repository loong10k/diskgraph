//! 共享 Engine 的 revision_reader 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError};
use diskgraph_core::{DiskGraph, ScopeId};
use diskgraph_store::SqliteSnapshotStore;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

impl Engine {
    /// 创建请求专用可信只读连接。
    /// 参数：无；使用既有一秒 SQLite 执行预算。
    /// 返回：独立 reader；调用方须完成资源授权。
    /// 请求专用只读连接，不持有共享写锁。
    pub fn revision_reader(&self) -> Result<SqliteSnapshotStore, EngineError> {
        Ok(SqliteSnapshotStore::open_reader(
            &self.graph_path,
            1000,
            None,
        )?)
    }
}

impl Engine {
    /// 创建绑定会话取消的可信只读连接。
    /// 参数：cancel 为会话取消标志。
    /// 返回：独立 reader 或打开失败；不持有共享写锁。
    /// 创建带会话取消标志的独立只读连接；关闭会话会中断 SQLite 执行。
    pub fn revision_reader_with_cancel(
        &self,
        cancel: Arc<AtomicBool>,
    ) -> Result<SqliteSnapshotStore, EngineError> {
        Ok(SqliteSnapshotStore::open_reader(
            &self.graph_path,
            1000,
            Some(cancel),
        )?)
    }
}

impl Engine {
    /// 创建继承整次请求期限和会话取消的可信只读连接。
    /// 参数：cancel 为会话取消标志，deadline 在最外层首次准备前生成。
    /// 返回：独立 reader 或打开失败；调用方负责实际资源授权，不持共享图写锁。
    pub fn revision_reader_with_cancel_until(
        &self,
        cancel: Arc<AtomicBool>,
        deadline: std::time::Instant,
    ) -> Result<SqliteSnapshotStore, EngineError> {
        Ok(SqliteSnapshotStore::open_reader_until(
            &self.graph_path,
            deadline,
            Some(cancel),
        )?)
    }
}

impl Engine {
    /// 可信内部查找范围的最新 revision。
    /// 参数：scope_id 为注册范围；调用方负责请求授权。
    /// 返回：可选 revision 或注册/读取失败。
    /// The latest published revision for a scope, if it has ever published.
    pub fn latest_revision(&self, scope_id: &ScopeId) -> Result<Option<String>, EngineError> {
        self.control()?.scope(scope_id)?;
        let server = self.server_id()?;
        Ok(self
            .revision_reader()?
            .latest_revision_for_scope(server.as_str(), scope_id.as_str())?)
    }
}

impl Engine {
    /// 隐式最新 revision 解析使用整次请求的 SQLite 期限。
    /// 参数：scope_id 为真实注册范围，deadline 在首次准备前生成；调用方负责请求授权。
    /// 返回：可选最新 revision 或范围/读取失败；控制锁与SQL继承原期限，缺失持久身份失败关闭。
    pub fn latest_revision_until(
        &self,
        scope_id: &ScopeId,
        deadline: std::time::Instant,
    ) -> Result<Option<String>, EngineError> {
        let control = self.control_until(deadline)?;
        let server = control
            .with_read_deadline(deadline, |control| {
                control.scope(scope_id)?;
                control.existing_server_id()
            })
            .map_err(|error| match error {
                diskgraph_store::StoreError::BudgetExceeded => {
                    EngineError::Business(diskgraph_core::BusinessError::BudgetExceeded)
                }
                other if other.is_interrupted() || other.is_busy() => {
                    EngineError::Business(diskgraph_core::BusinessError::BudgetExceeded)
                }
                other => EngineError::Store(other),
            })?;
        drop(control);
        Ok(
            SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?
                .latest_revision_for_scope(server.as_str(), scope_id.as_str())?,
        )
    }

    /// 可信内部加载完整旧图兼容结果。
    /// 参数：revision_id 为历史标识；调用方须先授权。
    /// 返回：完整 DiskGraph 或存储失败。
    /// Loads the v1 graph behind one published revision.
    pub fn load_revision(&self, revision_id: &str) -> Result<DiskGraph, EngineError> {
        Ok(self.graph()?.load_revision(revision_id)?)
    }
}
