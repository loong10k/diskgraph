//! ops_authorization：既有文件操作职责的原生 Rust 实现。
use crate::ops_error::OpsError;
use crate::path_codec::canonical_dir;
use crate::path_codec::path_of;
use crate::path_revalidation::revalidate_below;
use crate::side::Side;
use diskgraph_core::Authorizer;
use diskgraph_core::Decision;
use diskgraph_core::FileActionKind;
use diskgraph_core::Permission;
use diskgraph_core::PrincipalId;
use diskgraph_core::ScopeId;
use diskgraph_engine::Engine;
use std::path::Path;
use std::path::PathBuf;

/// 复核作用域的动作权限与撤销状态。
/// 参数：engine 为共享引擎；scope_id、principal、action 指定授权对象与动作。
/// 返回：允许则 Ok；否则为范围或授权错误。
pub(super) fn require_action(
    engine: &Engine,
    scope_id: &ScopeId,
    principal: &PrincipalId,
    action: FileActionKind,
) -> Result<(), OpsError> {
    if engine.scope(scope_id)?.revoked {
        return Err(OpsError::NotAuthorized("scope is revoked".into()));
    }
    if !matches!(
        engine
            .policy_authorizer()?
            .decide(principal, &Permission::FileAction(action), scope_id),
        Decision::Allowed
    ) {
        return Err(OpsError::NotAuthorized(format!(
            "{} action grant is required in scope {scope_id}",
            action_name(action)
        )));
    }
    Ok(())
}

/// 复核目标路径所属可写范围与主体权限。
/// 参数：engine 为引擎；directory 为目标目录；principal 和 action 指定主体及动作。
/// 返回：拥有该目标的最具体已授权范围根路径，或拒绝错误。
/// The most specific registered scope owns a destination. A writable path
/// outside the registry never inherits the source scope's permission.
pub(super) fn require_destination(
    engine: &Engine,
    directory: &Path,
    principal: &PrincipalId,
    action: FileActionKind,
) -> Result<PathBuf, OpsError> {
    let directory = canonical_dir(directory);
    if directory
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(OpsError::NotAuthorized(
            "destination contains an unresolved parent component".into(),
        ));
    }
    let scopes = engine.control_store()?.list_scopes()?;
    let owner = scopes
        .into_iter()
        .filter_map(|scope| {
            let root = path_of(&scope.root)?;
            directory
                .starts_with(&root)
                .then_some((root, scope.scope_id))
        })
        .max_by_key(|(root, _)| root.components().count())
        .ok_or_else(|| {
            OpsError::NotAuthorized("destination is outside registered scopes".into())
        })?;
    require_action(engine, &owner.1, principal, action)?;
    revalidate_below(&owner.0, &directory, Side::Target)
        .map_err(|fault| OpsError::Stale(format!("destination: {fault}")))?;
    Ok(owner.0)
}

/// 取得持久化动作名称。
/// 参数：action 为文件动作。
/// 返回：原有动作字符串。
/// The stable action name used in digests and the store.
pub(super) fn action_name(action: FileActionKind) -> &'static str {
    match action {
        FileActionKind::Move => "move",
        FileActionKind::Copy => "copy",
        FileActionKind::Trash => "trash",
        FileActionKind::Restore => "restore",
        FileActionKind::Purge => "purge",
    }
}
