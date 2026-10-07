//! CLI du_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::output::envelope_line;
use crate::scan_jobs::wait_for_terminal;
use diskgraph_core::{Authorizer, BusinessError, PrincipalId};
use diskgraph_engine::{Engine, EngineError};
use std::path::PathBuf;

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
        Command::Du { paths, total } => {
            let mut rows: Vec<(PathBuf, u64, String)> = Vec::new();
            let mut failures = 0_usize;
            let mut first_business_failure = None;
            for path in paths {
                let canonical = match path.canonicalize() {
                    Ok(canonical) => canonical,
                    Err(_) => {
                        eprintln!(
                            "diskgraph: cannot access {}: No such file or directory",
                            path.display()
                        );
                        failures += 1;
                        continue;
                    }
                };
                let scope_id = match engine.register_scope(&canonical, principal, authorizer) {
                    Ok(scope_id) => scope_id,
                    Err(error) => {
                        eprintln!("diskgraph: {}: {error}", path.display());
                        first_business_failure.get_or_insert(error);
                        failures += 1;
                        continue;
                    }
                };
                // register_scope granted this principal scope-local rights in
                // the policy store; the in-memory authorizer is a snapshot
                // from process start, so reload it before indexing.
                let authorizer = &crate::local::LocalIdentity::load(engine)?;
                let job = match engine.index_scope(&scope_id, principal, authorizer) {
                    Ok(job) => job,
                    Err(error) => {
                        eprintln!("diskgraph: {}: {error}", path.display());
                        first_business_failure.get_or_insert(error);
                        failures += 1;
                        continue;
                    }
                };
                // 其他长期服务可能已认领任务；等待或接管该任务，不运行其他队列项。
                let completed = match engine.run_job(&job.job_id, "du") {
                    Ok(record) => Ok(record),
                    Err(EngineError::Store(diskgraph_store::StoreError::Conflict(_)))
                    | Err(EngineError::Store(diskgraph_store::StoreError::StaleOwner)) => {
                        wait_for_terminal(engine, &job.job_id, "du")
                    }
                    Err(error) => Err(error),
                };
                match completed {
                    Ok(record) if record.state == diskgraph_store::JobState::Completed => {}
                    Ok(record) => {
                        eprintln!(
                            "diskgraph: {}: job ended {:?}",
                            path.display(),
                            record.state
                        );
                        failures += 1;
                        continue;
                    }
                    Err(error) => {
                        eprintln!("diskgraph: {}: {error}", path.display());
                        first_business_failure.get_or_insert(error);
                        failures += 1;
                        continue;
                    }
                }
                let Some(revision) = engine.latest_revision(&scope_id)? else {
                    failures += 1;
                    continue;
                };
                let root = engine.revision_root_node(&revision)?;
                rows.push((canonical, root.subtree_bytes, scope_id.as_str().to_owned()));
            }
            if failures > 0 && rows.is_empty() {
                // 全部失败时保留实际授权或平台错误，不能把现存路径误报为不存在。
                return Err(first_business_failure
                    .unwrap_or(EngineError::Business(BusinessError::NotFound)));
            }
            let grand_total: u64 = rows.iter().map(|row| row.1).sum();
            let mut data = serde_json::Map::new();
            let _ = total;
            for (path, bytes, _) in &rows {
                data.insert(
                    path.display().to_string(),
                    serde_json::json!({ "bytes": bytes }),
                );
            }
            if !cli.json {
                // du -sh shape: size, tab, path - readable without a parser.
                for (path, bytes, _) in &rows {
                    println!(
                        "{}\t{}",
                        diskgraph_core::treemap::human_bytes(*bytes),
                        path.display()
                    );
                }
                if *total && rows.len() > 1 {
                    println!(
                        "{}\ttotal",
                        diskgraph_core::treemap::human_bytes(grand_total)
                    );
                }
                if failures > 0 {
                    eprintln!("diskgraph: {failures} path(s) could not be measured");
                }
                return Ok(());
            }
            let mut payload = serde_json::Map::new();
            payload.insert("sizes".to_owned(), serde_json::Value::Object(data));
            if *total && rows.len() > 1 {
                payload.insert("total_bytes".to_owned(), serde_json::json!(grand_total));
            }
            if failures > 0 {
                payload.insert("failed_paths".to_owned(), serde_json::json!(failures));
            }
            out.push(envelope_line(
                engine,
                Ok(serde_json::Value::Object(payload)),
            ));
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_denied_paths_preserve_permission_error() {
        use clap::Parser;
        struct Denied;
        impl diskgraph_core::Authorizer for Denied {
            fn decide(
                &self,
                _: &diskgraph_core::PrincipalId,
                _: &diskgraph_core::Permission,
                _: &diskgraph_core::ScopeId,
            ) -> diskgraph_core::Decision {
                diskgraph_core::Decision::Denied(diskgraph_core::DenyReason::NoMatchingGrant)
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let engine = diskgraph_engine::Engine::open(diskgraph_engine::EngineConfig {
            data_dir: dir.path().join("data"),
            ..Default::default()
        })
        .unwrap();
        let cli = crate::cli::Cli::parse_from(["diskgraph", "du", dir.path().to_str().unwrap()]);
        let principal = diskgraph_core::PrincipalId::new("denied-du").unwrap();
        let result = super::run(&engine, &cli, &principal, &Denied, &mut Vec::new());
        assert!(
            matches!(
                result,
                Err(diskgraph_engine::EngineError::Business(
                    diskgraph_core::BusinessError::PermissionDenied
                ))
            ),
            "permission failure became: {result:?}"
        );
    }
}
