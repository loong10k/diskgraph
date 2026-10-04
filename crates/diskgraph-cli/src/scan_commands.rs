//! CLI scan_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::output::envelope_line;
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
        } => crate::git_sync::run(engine, cli, principal, authorizer, out),
        Command::Status { job } => {
            // Git 状态由共同授权入口绑定实际 scope 和 revision，固定诊断不包含工具原文。
            // 非 Git 任务返回 None，继续原扫描状态字段及查询路径。
            if let Some(details) = engine.git_job_status_details(job, principal, authorizer)? {
                out.push(envelope_line(engine, Ok(details)));
                return Ok(());
            }
            let record = engine.job_status(job)?;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "job_id": record.job_id,
                    "state": format!("{:?}", record.state).to_ascii_lowercase(),
                    "scope_id": record.scope_id.as_str(),
                })),
            ));
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
