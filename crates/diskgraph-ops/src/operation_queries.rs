//! operation_queries：既有文件操作职责的原生 Rust 实现。
use crate::operation_view::OperationView;
use crate::ops_error::OpsError;
use diskgraph_core::PrincipalId;
use diskgraph_core::ScopeId;
use diskgraph_engine::Engine;

/// 组合操作记录与各项执行结果。
/// 参数：operation 为操作记录；control 为调用者已持有的控制库。
/// 返回：完整 OperationView 或存储错误。
pub(super) fn view_of(
    operation: &diskgraph_store::Operation,
    control: &diskgraph_store::ControlStore,
) -> Result<OperationView, OpsError> {
    let items = control.operation_items(&operation.operation_id)?;
    let completed = items
        .iter()
        .filter(|item| item.result != diskgraph_store::OperationItemResult::Pending)
        .count();
    Ok(OperationView {
        operation_id: operation.operation_id.clone(),
        plan_id: operation.plan_id.clone(),
        scope_id: operation.scope_id.clone(),
        state: operation.state,
        remaining: items.len() - completed,
        items,
        completed,
    })
}

/// 按范围列出属于调用主体的操作；这是可信库入口的主体过滤。
/// 参数：engine 提供控制库；scope_id 指定范围；principal 用于记录主体过滤；limit 指定最大记录数。
/// 返回：按主体过滤的操作视图列表，或控制库读取错误。
/// Lists a principal's operations for a scope, newest first (C26).
pub fn list_operations(
    engine: &std::sync::Arc<Engine>,
    scope_id: &ScopeId,
    principal: &PrincipalId,
    limit: u64,
) -> Result<Vec<OperationView>, OpsError> {
    let control = engine.control_store()?;
    let mut views = Vec::new();
    for operation in control.list_operations(scope_id, limit)? {
        // Another principal's operation is not visible here (SC-04).
        if &operation.principal != principal {
            continue;
        }
        views.push(view_of(&operation, &control)?);
    }
    Ok(views)
}

/// 读取调用主体可查看的指定操作。
/// 参数：engine、operation_id、principal 指定记录与主体。
/// 返回：属于该主体的操作视图；不存在、主体不符或存储失败返回错误。
/// Shows one operation the principal owns.
pub fn show_operation(
    engine: &std::sync::Arc<Engine>,
    operation_id: &str,
    principal: &PrincipalId,
) -> Result<OperationView, OpsError> {
    let control = engine.control_store()?;
    let operation = control.operation(operation_id)?;
    if &operation.principal != principal {
        return Err(OpsError::NotAuthorized(
            "the operation belongs to another principal".into(),
        ));
    }
    view_of(&operation, &control)
}

/// 申请取消已授权主体可管理的操作。
/// 参数：engine、operation_id、principal 指定目标操作与主体。
/// 返回：取消后的视图或既有终态视图；主体不符或存储失败返回错误。
/// Cancels an operation. Items that already ran keep their result; only the
/// remaining ones stop, and a finished operation is never rewritten (OP-09).
pub fn cancel_operation(
    engine: &std::sync::Arc<Engine>,
    operation_id: &str,
    principal: &PrincipalId,
) -> Result<OperationView, OpsError> {
    let mut control = engine.control_store()?;
    let operation = control.operation(operation_id)?;
    if &operation.principal != principal {
        return Err(OpsError::NotAuthorized(
            "the operation belongs to another principal".into(),
        ));
    }
    if operation.state.is_terminal() {
        // Cancelling finished work would rewrite history.
        return view_of(&operation, &control);
    }
    let items = control.operation_items(operation_id)?;
    for item in &items {
        if item.result == diskgraph_store::OperationItemResult::Pending {
            control.record_item_result(
                operation_id,
                item.item_index,
                diskgraph_store::OperationItemResult::Failed,
                "cancelled; executor stops at its next observation boundary",
                None,
            )?;
        }
    }
    control.advance_operation_state(operation_id, diskgraph_store::OperationState::Cancelled)?;
    let operation = control.operation(operation_id)?;
    view_of(&operation, &control)
}
