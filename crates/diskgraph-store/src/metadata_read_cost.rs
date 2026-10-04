//! 借用 SQLite 字段的事前成本，使用调用方同一账本；来源：Rust D42，不拥有执行状态。
use crate::{Result, StoreError};
use rusqlite::types::ValueRef;

/// 参数：借用字段集合；返回：包含数值宽度的 checked 原始字节，不先复制字段。
pub(crate) fn raw(fields: &[ValueRef<'_>]) -> Result<usize> {
    fields.iter().try_fold(0usize, |sum, field| {
        let bytes = match field {
            ValueRef::Null => 0,
            ValueRef::Integer(_) | ValueRef::Real(_) => 8,
            ValueRef::Text(v) | ValueRef::Blob(v) => v.len(),
        };
        sum.checked_add(bytes).ok_or(StoreError::BudgetExceeded)
    })
}

/// 参数：真实原始字段、其中需要树/字符串解码的 JSON 长度和固定对象空间；返回：保守分配准入量。
/// 当前固定 serde 模型的 Value/容器、字段克隆、JSON scratch/digest 编码按每源字节 256 计费；
/// 其他字段复制/原生定位转换按四倍，固定结构另计。它是保守额度，不是 RSS/SQLite C 分配测量。
pub(crate) fn allocation(raw: usize, json: usize, fixed: usize) -> Result<u64> {
    let bytes = raw
        .checked_mul(4)
        .and_then(|v| json.checked_mul(256).and_then(|j| v.checked_add(j)))
        .and_then(|v| v.checked_add(fixed))
        .ok_or(StoreError::BudgetExceeded)?;
    u64::try_from(bytes).map_err(|_| StoreError::BudgetExceeded)
}

/// 参数：调用方同一准入回调及本行成本；返回：事前准入或原错误，拒绝后绝不拥有/解码。
pub(crate) fn charge(
    admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
    raw: usize,
    json: usize,
    fixed: usize,
    entries: u64,
) -> Result<()> {
    admit(
        u64::try_from(raw).map_err(|_| StoreError::BudgetExceeded)?,
        entries,
        allocation(raw, json, fixed)?,
    )
}
