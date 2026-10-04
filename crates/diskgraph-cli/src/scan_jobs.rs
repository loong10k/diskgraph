//! CLI scan_jobs 的真实职责实现。
use crate::output::envelope_line;
use diskgraph_core::{Authorizer, BusinessError, PrincipalId, ScopeId};
use diskgraph_engine::{Engine, EngineError};

/// 保留 run_scan 的原生业务职责与错误语义。来源：DiskGraph CLI main::run_scan。
/// 参数：与原入口的 run_scan 请求及执行依赖相同。返回：原业务结果或真实执行错误。
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_scan(
    engine: &Engine,
    scope: &str,
    sync: bool,
    wait: bool,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    out: &mut Vec<String>,
) -> Result<(), EngineError> {
    let scope_id = ScopeId::new(scope.to_owned())
        .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
    let job = if sync {
        engine.sync_scope(&scope_id, principal, authorizer)?
    } else {
        engine.index_scope(&scope_id, principal, authorizer)?
    };
    if !wait {
        out.push(envelope_line(
            engine,
            Ok(serde_json::json!({ "job_id": job.job_id, "state": "queued" })),
        ));
        return Ok(());
    }
    let owner = format!("cli-{principal}");
    // 其他长期服务可能认领任务；--wait 必须核验该 owner 的实际终态。
    let finished = match engine.run_job(&job.job_id, &owner) {
        Ok(record) => record,
        Err(EngineError::Store(diskgraph_store::StoreError::Conflict(_)))
        | Err(EngineError::Store(diskgraph_store::StoreError::StaleOwner)) => {
            wait_for_terminal(engine, &job.job_id, &owner)?
        }
        Err(error) => return Err(error),
    };
    ensure_completed(&finished)?;
    let revision = engine.revision_for_job(&finished.job_id, principal, authorizer)?;
    out.push(envelope_line(
        engine,
        Ok(serde_json::json!({
            "job_id": finished.job_id,
            "state": format!("{:?}", finished.state).to_ascii_lowercase(),
            "revision_id": revision,
        })),
    ));
    Ok(())
}

/// 保留 wait_for_terminal 的原生业务职责与错误语义。来源：DiskGraph CLI main::wait_for_terminal。
/// 参数：与原入口的 wait_for_terminal 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn wait_for_terminal(
    engine: &Engine,
    job_id: &str,
    owner: &str,
) -> Result<diskgraph_store::JobRecord, EngineError> {
    wait_for_terminal_until(
        engine,
        job_id,
        owner,
        std::time::Instant::now() + std::time::Duration::from_secs(120),
    )
}

/// 保留 wait_for_terminal_until 的原生业务职责与错误语义。来源：DiskGraph CLI main::wait_for_terminal_until。
/// 参数：与原入口的 wait_for_terminal_until 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn wait_for_terminal_until(
    engine: &Engine,
    job_id: &str,
    owner: &str,
    deadline: std::time::Instant,
) -> Result<diskgraph_store::JobRecord, EngineError> {
    use diskgraph_store::JobState;
    loop {
        let record = engine.job_status(job_id)?;
        if matches!(
            record.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            ensure_completed(&record)?;
            return Ok(record);
        }
        if std::time::Instant::now() >= deadline {
            return Err(EngineError::Business(BusinessError::Timeout));
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if record.state == JobState::Queued
            || (record.state == JobState::Running && record.lease_expires_unix_ms <= now_ms)
        {
            let settled = engine.settle_expired_job(job_id)?;
            if matches!(
                settled.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            ) {
                ensure_completed(&settled)?;
                return Ok(settled);
            }
            // 数据库条件更新与 fencing 决定唯一 owner；存活 owner 不被抢占。
            // 认领成功后的扫描仍使用 Engine 扫描预算，deadline 仅限制等待。
            match engine.run_job(job_id, owner) {
                Ok(record) => {
                    ensure_completed(&record)?;
                    return Ok(record);
                }
                Err(EngineError::Store(diskgraph_store::StoreError::Conflict(_)))
                | Err(EngineError::Store(diskgraph_store::StoreError::StaleOwner)) => {}
                Err(error) => return Err(error),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// 保留 ensure_completed 的原生业务职责与错误语义。来源：DiskGraph CLI main::ensure_completed。
/// 参数：与原入口的 ensure_completed 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn ensure_completed(record: &diskgraph_store::JobRecord) -> Result<(), EngineError> {
    match record.state {
        diskgraph_store::JobState::Completed => Ok(()),
        diskgraph_store::JobState::Failed | diskgraph_store::JobState::Cancelled => {
            Err(EngineError::Business(BusinessError::Partial))
        }
        _ => Err(EngineError::Business(BusinessError::Conflict)),
    }
}
