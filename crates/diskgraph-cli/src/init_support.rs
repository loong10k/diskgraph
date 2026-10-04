//! CLI init_support 的真实职责实现。
use crate::installer;
use crate::scan_jobs::ensure_completed;
use crate::scan_jobs::wait_for_terminal;
use diskgraph_core::{Authorizer, BusinessError, PrincipalId};
use diskgraph_engine::{Engine, EngineError};
use std::path::{Path, PathBuf};

/// 保留 unsafe_root_reason 的原生业务职责与错误语义。来源：DiskGraph CLI main::unsafe_root_reason。
/// 参数：与原入口的 unsafe_root_reason 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn unsafe_root_reason(root: &Path) -> Option<String> {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && root == home
    {
        return Some(format!("the home directory {}", home.display()));
    }
    if root.parent().is_none() {
        return Some(format!("the filesystem root {}", root.display()));
    }
    None
}

/// 保留 absolute_data_dir 的原生业务职责与错误语义。来源：DiskGraph CLI main::absolute_data_dir。
/// 参数：与原入口的 absolute_data_dir 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn absolute_data_dir(data_dir: &Path) -> PathBuf {
    if data_dir.is_absolute() {
        return data_dir.to_path_buf();
    }
    std::env::current_dir().map_or_else(|_| data_dir.to_path_buf(), |cwd| cwd.join(data_dir))
}

/// 保留 init_index 的原生业务职责与错误语义。来源：DiskGraph CLI main::init_index。
/// 参数：与原入口的 init_index 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn init_index(
    engine: &Engine,
    root: &Path,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
) -> Result<serde_json::Value, EngineError> {
    let started = std::time::Instant::now();
    let scope_id = engine.register_scope(root, principal, authorizer)?;
    // register_scope granted this principal scope-local rights in the policy
    // store; the in-memory authorizer is a snapshot from process start, so
    // reload it before asking for the job - otherwise the scan is refused by
    // the very scope it is about to measure.
    let authorizer = &crate::local::LocalIdentity::load(engine)?;
    // A second init is a rescan, not a second scope: the same root maps to
    // the same scope id, and index_scope is what republishes it.
    let job = engine.index_scope(&scope_id, principal, authorizer)?;
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
    let revision = Some(engine.revision_for_job(&finished.job_id, principal, authorizer)?);
    let root_node = revision
        .as_deref()
        .map(|revision| engine.revision_root_node(revision))
        .transpose()?;
    let index_bytes = std::fs::metadata(engine.data_dir().join("diskgraph.sqlite"))
        .map(|meta| meta.len())
        .unwrap_or(0);
    Ok(serde_json::json!({
        "root": root.display().to_string(),
        "data_dir": engine.data_dir().display().to_string(),
        "scope_id": scope_id.as_str(),
        "revision_id": revision,
        "state": format!("{:?}", finished.state).to_ascii_lowercase(),
        "files": root_node.as_ref().map(|node| node.files),
        "directories": root_node.as_ref().map(|node| node.directories),
        "subtree_bytes": root_node.as_ref().map(|node| node.subtree_bytes),
        "index_bytes": index_bytes,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    }))
}

/// 保留 resolve_targets 的原生业务职责与错误语义。来源：DiskGraph CLI main::resolve_targets。
/// 参数：与原入口的 resolve_targets 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn resolve_targets(requested: &[String]) -> Result<Vec<installer::Target>, EngineError> {
    let wanted = if requested.is_empty() {
        vec!["auto".to_owned()]
    } else {
        requested.to_vec()
    };
    let mut out: Vec<installer::Target> = Vec::new();
    for value in &wanted {
        match value.to_ascii_lowercase().as_str() {
            "auto" => out.extend(
                installer::Target::ALL
                    .into_iter()
                    .filter(|target| target.detected()),
            ),
            "all" => out.extend(installer::Target::ALL),
            "none" => {}
            other => out.extend(installer::Target::parse_list(other).map_err(|message| {
                eprintln!("diskgraph: {message}");
                EngineError::Business(BusinessError::InvalidArgument)
            })?),
        }
    }
    out.dedup();
    Ok(out)
}
