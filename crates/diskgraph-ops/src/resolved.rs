//! resolved：既有文件操作职责的原生 Rust 实现。
use crate::ops_error::OpsError;
use std::collections::HashSet;
use std::path::PathBuf;

/// 计划节点解析结果的节点编号、路径、可选身份和字节数四元组。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::Resolved`，保留既有语义。
/// A requested object resolved to a live path, its identity, and its size.
pub(super) type Resolved = (u64, PathBuf, Option<String>, u64);

/// 去除包含其他已选对象的祖先对象并检测重复路径。
/// 参数：items 为解析后的对象集合。
/// 返回：互不嵌套的对象列表或重叠错误。
/// Drops any item that is an ancestor of another, so a plan never moves the
/// same bytes twice. Ancestor is decided by real path containment, not by name.
pub(super) fn drop_nested(items: &[Resolved]) -> Result<Vec<Resolved>, OpsError> {
    let mut keep: Vec<Resolved> = Vec::new();
    for candidate in items {
        let is_ancestor_of_other = items
            .iter()
            .any(|other| other.1 != candidate.1 && other.1.starts_with(&candidate.1));
        if is_ancestor_of_other {
            // Dropping the parent is safe: its children cover the same bytes.
            continue;
        }
        keep.push(candidate.clone());
    }
    if keep.is_empty() {
        return Err(OpsError::OverlappingObjects(items[0].0));
    }
    // Two items that are the same path would double-count; that is an overlap
    // we cannot resolve by dropping one, so refuse.
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for item in &keep {
        if !seen.insert(item.1.clone()) {
            return Err(OpsError::OverlappingObjects(item.0));
        }
    }
    Ok(keep)
}
