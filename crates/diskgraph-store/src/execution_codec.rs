use crate::{Plan, StoreError};
use diskgraph_core::FileActionKind;

/// Milliseconds since the epoch, used for every control-plane timestamp.
/// 生成持久时间或非法状态诊断。
/// 参数：无额外输入；实例方法使用当前连接/记录。
/// 返回：Unix 毫秒时间。
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 生成持久时间或非法状态诊断。
/// 参数：table：诊断中的状态表名称；value：待编码/解析字段。
/// 返回：带表名/非法标签的 InvalidGraph。
pub(crate) fn bad_state(table: &str, value: &str) -> StoreError {
    StoreError::InvalidGraph(format!("{table} has an unknown state {value:?}"))
}

/// 编码或解析既有稳定 wire 标签。
/// 参数：action：独立文件动作能力。
/// 返回：move/copy/trash/restore/purge 稳定动作标签。
pub(crate) fn action_name(action: FileActionKind) -> &'static str {
    match action {
        FileActionKind::Move => "move",
        FileActionKind::Copy => "copy",
        FileActionKind::Trash => "trash",
        FileActionKind::Restore => "restore",
        FileActionKind::Purge => "purge",
    }
}

/// Lossless locator keys are hex-encoded path bytes. Component boundaries are
/// the encoded separator, so /a and /ab do not spuriously overlap.
/// 按对象身份和编码路径组件边界处理资源冲突键。
/// 参数：left：编码路径组件或身份冲突键；right：另一资源冲突键。
/// 返回：两个编码组件/身份键是否重叠。
pub(crate) fn key_overlap(left: &str, right: &str) -> bool {
    // Inode claims are exact; path claims retain component-aware ancestry.
    // Prefixing identity claims keeps them disjoint from hex path keys.
    if left.starts_with("identity:") || right.starts_with("identity:") {
        return left == right;
    }
    left == right
        || left == "2f"
        || right == "2f"
        || left
            .strip_prefix(right)
            .is_some_and(|tail| tail.starts_with("2f"))
        || right
            .strip_prefix(left)
            .is_some_and(|tail| tail.starts_with("2f"))
}

/// 按对象身份和编码路径组件边界处理资源冲突键。
/// 参数：plan：精确不可变计划。
/// 返回：精确源身份和目标组件的冲突键集合。
pub(crate) fn claim_keys(plan: &Plan) -> Vec<String> {
    let mut keys = Vec::with_capacity(plan.items.len() * 3);
    for item in &plan.items {
        keys.push(item.locator_key.clone());
        if let Some(identity) = item.identity.as_deref() {
            keys.push(format!("identity:{identity}"));
        }
        if let Some(directory) = &plan.target_locator_key {
            let basename_start = item
                .locator_key
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .rposition(|chunk| chunk == b"2f")
                .map(|index| (index + 1) * 2)
                .unwrap_or(0);
            let basename = &item.locator_key[basename_start..];
            let separator = if directory.ends_with("2f") { "" } else { "2f" };
            keys.push(format!("{directory}{separator}{basename}"));
        }
    }
    keys
}

/// 编码或解析既有稳定 wire 标签。
/// 参数：value：待编码/解析字段。
/// 返回：已知标签对应 FileActionKind，未知为 None。
pub(crate) fn action_from_name(value: &str) -> Option<FileActionKind> {
    Some(match value {
        "move" => FileActionKind::Move,
        "copy" => FileActionKind::Copy,
        "trash" => FileActionKind::Trash,
        "restore" => FileActionKind::Restore,
        "purge" => FileActionKind::Purge,
        _ => return None,
    })
}
