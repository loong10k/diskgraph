//! 启动与运行期注册共用原始根隔离规则；来源：SC-01 历史 revision 授权合同。
use crate::EngineError;
use diskgraph_core::{ResourceLocator, ServerId};
use diskgraph_store::{ScopeRecord, SqliteSnapshotStore};
use std::collections::HashSet;

/// 参数：graph 为串行写库，server/scopes 为同一控制库注册表；返回：可无损回填的根。
/// 保留原审计归属；仅隔离无法证明原始身份的已有 revision，不发布新数据。
pub(super) fn eligible_roots(
    graph: &mut SqliteSnapshotStore,
    server_id: &ServerId,
    scopes: &[ScopeRecord],
) -> Result<Vec<(String, ResourceLocator)>, EngineError> {
    let mut unverifiable_roots = HashSet::new();
    let mut roots = scopes
        .iter()
        .map(|scope| {
            let root = match scope.root.kind {
                diskgraph_core::LocatorKind::NativePath => {
                    ResourceLocator::NativePath(scope.root.display().to_owned())
                }
                diskgraph_core::LocatorKind::DocumentUri => {
                    ResourceLocator::DocumentUri(scope.root.display().to_owned())
                }
            };
            // 旧快照仅有字符串：先证明注册根的原始身份能无损表示为该字符串。
            // 非 UTF-8、外平台不可解码或损坏定位不能凭显示别名获得旧 revision。
            let lossless = match scope.root.kind {
                diskgraph_core::LocatorKind::NativePath => scope
                    .root
                    .to_native_path()
                    .ok()
                    .and_then(|path| path.to_str().map(str::to_owned))
                    .is_some_and(|path| path == scope.root.display()),
                diskgraph_core::LocatorKind::DocumentUri => scope
                    .root
                    .raw_bytes()
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .is_some_and(|uri| uri == scope.root.display()),
            };
            if !lossless {
                unverifiable_roots.insert(root.clone());
            }
            (scope.scope_id.as_str().to_owned(), root)
        })
        .collect::<Vec<_>>();
    // 拒绝整个有损别名组，不能删除冲突候选后人为制造“唯一匹配”。
    for scope in scopes {
        let display_root = match scope.root.kind {
            diskgraph_core::LocatorKind::NativePath => {
                ResourceLocator::NativePath(scope.root.display().to_owned())
            }
            diskgraph_core::LocatorKind::DocumentUri => {
                ResourceLocator::DocumentUri(scope.root.display().to_owned())
            }
        };
        if unverifiable_roots.contains(&display_root) {
            let native = match scope.root.kind {
                diskgraph_core::LocatorKind::NativePath => {
                    scope.root.to_native_path().ok().and_then(|path| {
                        diskgraph_core::QualifiedLocator::from_native_path(&path).ok()
                    })
                }
                diskgraph_core::LocatorKind::DocumentUri => scope
                    .root
                    .raw_bytes()
                    .ok()
                    .and_then(|raw| String::from_utf8(raw).ok())
                    .and_then(|uri| diskgraph_core::QualifiedLocator::from_document_uri(uri).ok()),
            };
            graph.isolate_unconfirmed_revision_roots(
                server_id.as_str(),
                scope.scope_id.as_str(),
                native.as_ref(),
            )?;
        }
    }
    roots.retain(|(_, root)| !unverifiable_roots.contains(root));
    Ok(roots)
}
