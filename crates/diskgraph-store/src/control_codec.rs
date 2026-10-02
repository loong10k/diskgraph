use diskgraph_core::{FileActionKind, LocatorKind, Permission};

/// 编码或解析既有稳定 wire 标签。
/// 参数：kind：节点或任务类型。
/// 返回：native_path 或 document_uri 稳定标签。
pub(crate) fn locator_kind_tag(kind: LocatorKind) -> &'static str {
    match kind {
        LocatorKind::NativePath => "native_path",
        LocatorKind::DocumentUri => "document_uri",
    }
}

/// 编码或解析既有稳定 wire 标签。
/// 参数：value：待编码/解析字段。
/// 返回：document_uri 还原文档定位，其他值保持旧 NativePath fallback。
pub(crate) fn parse_locator_kind(value: &str) -> LocatorKind {
    match value {
        "document_uri" => LocatorKind::DocumentUri,
        _ => LocatorKind::NativePath,
    }
}

/// 编码或解析既有稳定 wire 标签。
/// 参数：value：待编码/解析字段。
/// 返回：已知能力对应 Permission，未知为 None。
pub(crate) fn parse_permission(value: &str) -> Option<Permission> {
    Some(match value {
        "metadata:read" => Permission::MetadataRead,
        "content:read" => Permission::ContentRead,
        "index:write" => Permission::IndexWrite,
        "scope:admin" => Permission::ScopeAdmin,
        "operations:view" => Permission::OperationView,
        "files:move" => Permission::FileAction(FileActionKind::Move),
        "files:copy" => Permission::FileAction(FileActionKind::Copy),
        "files:trash" => Permission::FileAction(FileActionKind::Trash),
        "files:restore" => Permission::FileAction(FileActionKind::Restore),
        "files:purge" => Permission::FileAction(FileActionKind::Purge),
        _ => return None,
    })
}
