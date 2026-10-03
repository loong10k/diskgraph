//! SQLite 借用观测列的一致性验证，调用方负责在解码前准入预算。

use crate::{Result, StoreError, StoredWindowsObservation};
use diskgraph_core::{WindowsFileObservation, WindowsObservationGap};
use rusqlite::types::ValueRef;

/// 从借用列解码完整观测或固定缺失原因，不转换宿主路径。
/// 参数：format/raw/gap 为同一行观测列，kind/encoding 为对应定位标签。
/// 返回：有效观测或缺失原因；半字段、类型或协议损坏明确报错。
pub(crate) fn decode(
    format: ValueRef<'_>,
    raw: ValueRef<'_>,
    gap: ValueRef<'_>,
    kind: ValueRef<'_>,
    encoding: ValueRef<'_>,
) -> Result<StoredWindowsObservation> {
    let (observation, gap) = match (format, raw, gap) {
        (ValueRef::Null, ValueRef::Null, ValueRef::Null) => {
            (None, Some(WindowsObservationGap::NotCaptured))
        }
        (ValueRef::Null, ValueRef::Null, ValueRef::Text(code)) => {
            let code =
                std::str::from_utf8(code).map_err(|_| invalid("observation gap is not UTF-8"))?;
            let gap =
                WindowsObservationGap::parse(code).map_err(|error| invalid(&error.to_string()))?;
            (None, Some(gap))
        }
        (ValueRef::Text(format), ValueRef::Blob(raw), ValueRef::Null) => {
            if format != WindowsFileObservation::FORMAT_LABEL.as_bytes() {
                return Err(invalid("unknown native observation format"));
            }
            if kind != ValueRef::Text(b"native_path")
                || encoding != ValueRef::Text(b"windows_utf16_le")
            {
                return Err(invalid(
                    "Windows observation requires a Windows native locator",
                ));
            }
            let observation =
                WindowsFileObservation::decode(raw).map_err(|error| invalid(&error.to_string()))?;
            (Some(observation), None)
        }
        _ => return Err(invalid("partial or invalid native observation columns")),
    };
    Ok(StoredWindowsObservation { observation, gap })
}
fn invalid(reason: &str) -> StoreError {
    StoreError::InvalidGraph(reason.into())
}
