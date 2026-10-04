//! C03 Git 请求接线。来源：原生 CLI 与 Engine 持久采集入口；不构造执行命令或客户端预算。
use crate::cli::Cli;
use crate::command::Command;
use crate::output::envelope_line;
use crate::scan_jobs::{ensure_completed, wait_for_terminal};
use diskgraph_core::{Authorizer, BusinessError, JobRequestAuthority, PrincipalId, ScopeId};
use diskgraph_engine::{Engine, EngineError};

/// 将显式 Git 目标入队，并按 --wait 获取实际发布回执的 revision。
/// 参数：engine 为共享引擎，cli 为已解析请求，principal 为真实本地主体，authorizer 为当前策略，out 为响应缓冲。
/// 返回：真实持久任务响应或业务拒绝；扫描 sync 由既有处理器执行。
pub(crate) fn run(
    engine: &Engine,
    cli: &Cli,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    out: &mut Vec<String>,
) -> Result<(), EngineError> {
    let Command::Sync {
        scope,
        wait,
        collector: Some(collector),
        revision: Some(revision),
        node_id: Some(node_id),
    } = &cli.command
    else {
        return Err(BusinessError::InvalidArgument.into());
    };
    if collector != "git" {
        return Err(BusinessError::InvalidArgument.into());
    }
    let scope = ScopeId::new(scope.clone()).map_err(|_| BusinessError::InvalidArgument)?;
    // 本地 CLI 明确绑定实际主体；预算和固定 Git 命令由服务端创建，不来自参数。
    let authority = JobRequestAuthority::trusted_local(principal.clone(), "cli")?;
    let job = engine
        .git_evidence_scope_with_authority(&scope, revision, *node_id, &authority, authorizer)?;
    if !*wait {
        out.push(envelope_line(
            engine,
            Ok(serde_json::json!({
                "job_id": job.job_id,
                "state": format!("{:?}", job.state).to_ascii_lowercase(),
            })),
        ));
        return Ok(());
    }
    let owner = format!("cli-{principal}");
    let finished = match engine.run_job(&job.job_id, &owner) {
        Ok(record) => record,
        Err(EngineError::Store(diskgraph_store::StoreError::Conflict(_)))
        | Err(EngineError::Store(diskgraph_store::StoreError::StaleOwner)) => {
            wait_for_terminal(engine, &job.job_id, &owner)?
        }
        Err(error) => return Err(error),
    };
    ensure_completed(&finished)?;
    // 已提交结果来自真实 job receipt，不能根据 job ID、latest 或当前 fence 拼接。
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
