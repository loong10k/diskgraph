//! CLI comparison_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::compare_support::plan_to_json;
use crate::compare_support::resolve_side;
use crate::snapshot_reply;
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
        Command::Compare {
            from,
            to,
            from_scope,
            to_scope,
            plan,
            method,
            only_differences,
            limit,
            tolerance,
            verify_content,
            verify_files,
            verify_bytes_per_file,
        } => {
            let (from_revision, from_scope_id) = resolve_side(
                engine,
                from.as_deref(),
                from_scope.as_deref(),
                "from",
                deadline,
            )?;
            let (to_revision, to_scope_id) =
                resolve_side(engine, to.as_deref(), to_scope.as_deref(), "to", deadline)?;

            // Verification needs a scope on each side: the content grant is
            // granted per scope, and reading a path is only allowed inside the
            // scope that covers it.
            // Content is read inside a scope, so verification needs one on
            // each side; a bare revision carries no grant to check against.
            if *verify_content && (from_scope_id.is_none() || to_scope_id.is_none()) {
                eprintln!(
                    "diskgraph: --verify-content needs --from-scope and --to-scope: \
                     content is read inside a scope, not from a bare revision"
                );
                return Err(EngineError::Business(BusinessError::InvalidArgument));
            }
            engine.authorize_revision_until(
                from_scope_id.as_ref(),
                &from_revision,
                principal,
                authorizer,
                deadline,
            )?;
            engine.authorize_revision_until(
                to_scope_id.as_ref(),
                &to_revision,
                principal,
                authorizer,
                deadline,
            )?;
            let mut verification = None;

            if *plan {
                let sync_method = diskgraph_core::SyncMethod::parse(method)
                    .ok_or(EngineError::Business(BusinessError::InvalidArgument))?;
                let sync = engine.sync_plan_until(
                    &from_revision,
                    &to_revision,
                    sync_method,
                    *tolerance,
                    snapshot_reply::budget(),
                    principal,
                    authorizer,
                    deadline,
                )?;
                out.push(snapshot_reply::finish_plan(
                    engine,
                    principal,
                    authorizer,
                    &[&from_revision, &to_revision],
                    plan_to_json(&sync, *limit),
                    deadline,
                )?);
                return Ok(());
            }

            let mut report = engine.compare_revisions_until(
                &from_revision,
                &to_revision,
                *tolerance,
                snapshot_reply::budget(),
                principal,
                authorizer,
                deadline,
            )?;
            if *verify_content {
                let budget = diskgraph_engine::verify::VerifyBudget {
                    max_files: *verify_files,
                    max_bytes_per_file: *verify_bytes_per_file,
                };
                let (promoted, summary) = diskgraph_engine::verify::verify_same_rows_until(
                    engine,
                    report,
                    from_scope_id.as_ref().expect("checked above"),
                    to_scope_id.as_ref().expect("checked above"),
                    principal,
                    authorizer,
                    budget,
                    deadline,
                )?;
                report = promoted;
                verification = Some(summary);
            }
            let mut json = report.to_json(Some(*limit));
            if *only_differences {
                let mut shown = 0_usize;
                if let Some(rows) = json
                    .get_mut("rows")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    rows.retain(|row| {
                        row.get("verdict")
                            .and_then(|verdict| verdict.get("status"))
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|status| status != "same")
                    });
                    rows.truncate(*limit);
                    shown = rows.len();
                }
                if let Some(object) = json.as_object_mut() {
                    object.insert("entries".into(), serde_json::json!(shown));
                }
            }
            if let (Some(summary), Some(object)) = (verification, json.as_object_mut()) {
                object.insert("verification".into(), serde_json::json!(summary));
            }
            out.push(snapshot_reply::finish(
                engine,
                principal,
                authorizer,
                &[&from_revision, &to_revision],
                json,
                deadline,
            )?);
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
