//! CLI scan_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::scan_jobs::run_scan;
use diskgraph_core::{Authorizer, BusinessError, PrincipalId};
use diskgraph_engine::{Engine, EngineError};

/// 执行本组真实业务命令，保持原授权、预算、响应与副作用顺序。
/// 参数：本请求的解析参数及对应 Engine 依赖。返回：执行成功或原业务错误。
pub(crate) fn run(
    engine: &Engine,
    cli: &Cli,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    out: &mut Vec<String>,
    deadline: std::time::Instant,
) -> Result<(), EngineError> {
    match &cli.command {
        Command::Index { scope, wait } => {
            run_scan(engine, scope, false, *wait, principal, authorizer, out)
        }
        Command::Sync {
            scope,
            wait,
            collector: None,
            ..
        } => run_scan(engine, scope, true, *wait, principal, authorizer, out),
        Command::Sync {
            collector: Some(_), ..
        } => crate::evidence_sync::run(engine, cli, principal, authorizer, out),
        Command::Status { job } => {
            let server = engine.server_id_until(deadline)?;
            let encoded = engine.with_job_status_authorized_until(job, principal, authorizer, deadline, |record| {
                // collector详情仍通过既有实际scope/revision授权服务，全部继承原期限。
                let data = if let Some(details) = engine.git_job_status_details_until(job, principal, authorizer, deadline)? {
                    details
                } else if let Some(details) = engine.process_job_status_details_until(job, principal, authorizer, deadline)? {
                    details
                } else {
                    serde_json::json!({ "job_id": record.job_id, "state": format!("{:?}", record.state).to_ascii_lowercase(), "scope_id": record.scope_id.as_str() })
                };
                crate::snapshot_reply::encode_data(server, &data, std::time::Instant::now() >= deadline)
            })?;
            out.push(encoded);
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
