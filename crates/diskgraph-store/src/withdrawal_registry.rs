use crate::control_database_identity::ControlDatabaseIdentity;
use crate::withdrawal_entry::WithdrawalEntry;
use crate::{AuthorizationWithdrawalWatch, ControlStore, Result, StoreError};
use diskgraph_core::{Permission, PrincipalId, ScopeId};
use std::sync::{Arc, Mutex, OnceLock, Weak, atomic::AtomicBool};

const MAX_PER_DATABASE: usize = 256;
const MAX_TOTAL: usize = 4096;
static REGISTRY: OnceLock<Mutex<WithdrawalRegistry>> = OnceLock::new();

/// 有界进程内 Weak 订阅表，不保留永久数据库桶，不持有数据库连接。
/// 来源：DiskGraph 原生 Rust D45 的实际 main 文件身份匹配与请求生命周期。
pub(crate) struct WithdrawalRegistry {
    entries: Vec<Weak<WithdrawalEntry>>,
}

impl WithdrawalRegistry {
    /// 建立空的有界 registry，独立测试实例不修改生产全局配额。
    /// 参数：无；返回：未分配请求条目的注册表。
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// 只在内存中登记已解析的依赖；所有 SQL/原生身份读取必须在取得锁之前完成。
    /// 参数：store 为原连接代次，principal/scope/permission 为真实依赖，policy_epoch/observed_generation 为锁外同一 SQL 快照的有效 epoch 与提交排序。
    /// 返回：一个负向 watch；未知原生身份为 None，达到 256/4096 或分配失败为 BudgetExceeded。
    pub(crate) fn register(
        &mut self,
        store: &ControlStore,
        principal: &PrincipalId,
        scope: &ScopeId,
        permission: &Permission,
        policy_epoch: Option<u64>,
        observed_generation: u64,
    ) -> Result<Option<AuthorizationWithdrawalWatch>> {
        let Some(identity) = store.withdrawal_incarnation.identity() else {
            return Ok(None);
        };
        self.entries
            .retain(|weak| weak.upgrade().is_some_and(|entry| entry.is_live()));
        let matching = self
            .entries
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|entry| entry.identity == identity)
            .count();
        if self.entries.len() >= MAX_TOTAL || matching >= MAX_PER_DATABASE {
            return Err(StoreError::BudgetExceeded);
        }
        self.entries
            .try_reserve(1)
            .map_err(|_| StoreError::BudgetExceeded)?;
        // ID 由 Core 限制为 64 字节；条目数量在这些小型复制与 Arc 分配之前准入。
        let entry = Arc::new(WithdrawalEntry {
            identity,
            incarnation: Arc::downgrade(&store.withdrawal_incarnation),
            principal: principal.clone(),
            scope: scope.clone(),
            permission: *permission,
            policy_epoch,
            observed_generation,
            withdrawn: AtomicBool::new(false),
        });
        self.entries.push(Arc::downgrade(&entry));
        Ok(Some(AuthorizationWithdrawalWatch { entry }))
    }

    fn publish_scope(
        &mut self,
        identity: ControlDatabaseIdentity,
        scope: &ScopeId,
        generation: u64,
    ) {
        self.entries.retain(|weak| {
            let Some(entry) = weak.upgrade() else {
                return false;
            };
            if !entry.is_live() {
                return false;
            }
            if entry.identity == identity
                && &entry.scope == scope
                && entry.observed_generation < generation
            {
                entry.withdraw();
            }
            true
        });
    }

    fn publish_grant(
        &mut self,
        identity: ControlDatabaseIdentity,
        principal: &PrincipalId,
        scope: &ScopeId,
        permission: &Permission,
        epoch: u64,
        generation: u64,
    ) {
        self.entries.retain(|weak| {
            let Some(entry) = weak.upgrade() else {
                return false;
            };
            if !entry.is_live() {
                return false;
            }
            if entry.identity == identity
                && &entry.principal == principal
                && &entry.scope == scope
                && entry.permission == *permission
                && entry.policy_epoch == Some(epoch)
                && entry.observed_generation < generation
            {
                entry.withdraw();
            }
            true
        });
    }
}

/// 在短锁内登记请求，既不执行 SQL/OS，也不调用授权或消费者代码。
/// 参数：固定原连接及锁外解析依赖；返回：原 register 的有界准入结果。
pub(crate) fn register(
    store: &ControlStore,
    principal: &PrincipalId,
    scope: &ScopeId,
    permission: &Permission,
    epoch: Option<u64>,
    observed_generation: u64,
) -> Result<Option<AuthorizationWithdrawalWatch>> {
    REGISTRY
        .get_or_init(|| Mutex::new(WithdrawalRegistry::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .register(
            store,
            principal,
            scope,
            permission,
            epoch,
            observed_generation,
        )
}

/// 提交后通知 scope 依赖；没有身份或没有订阅表时无需建立任何新共享状态。
/// 参数：store 为已提交的原连接，scope 为确实由未撤销变为撤销的范围；返回：无，仅发布负向事实。
pub(crate) fn publish_scope(store: &ControlStore, scope: &ScopeId, generation: u64) {
    // 即使没有跨连接身份，也保留原连接已提交的负向计数，不新增SQL或授予权限。
    store
        .withdrawal_incarnation
        .record_committed_generation(generation);
    let (Some(identity), Some(registry)) =
        (store.withdrawal_incarnation.identity(), REGISTRY.get())
    else {
        return;
    };
    registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .publish_scope(identity, scope, generation);
}

/// 提交后通知被实际删除的当前有效 grant，历史 epoch 与 DELETE0 不调用此函数。
/// 参数：store 为原连接，principal/scope/permission/epoch/generation 为同一事务确认的删除事实与最终计数；返回：无。
pub(crate) fn publish_grant(
    store: &ControlStore,
    principal: &PrincipalId,
    scope: &ScopeId,
    permission: &Permission,
    epoch: u64,
    generation: u64,
) {
    store
        .withdrawal_incarnation
        .record_committed_generation(generation);
    let (Some(identity), Some(registry)) =
        (store.withdrawal_incarnation.identity(), REGISTRY.get())
    else {
        return;
    };
    registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .publish_grant(identity, principal, scope, permission, epoch, generation);
}
