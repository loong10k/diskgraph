use crate::{CursorContext, CursorRejection, PagingCursor};
use base64::Engine;
use serde::{Deserialize, Serialize};

/// v2 搜索游标：授权上下文和实际 name/id keyset 位置不可混用旧 offset 游标。
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchCursor {
    pub version: u8,
    pub binding: PagingCursor,
    pub last_name: String,
    pub last_id: u64,
}

impl SearchCursor {
    /// 编码授权与排序绑定的 keyset 游标。
    pub fn encode(&self) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(self).expect("cursor fields serialize"))
    }
    /// 验证当前主体、scope、revision、过滤、排序和策略版本。
    pub fn decode(encoded: &str, context: &CursorContext<'_>) -> Result<Self, CursorRejection> {
        if encoded.len() > 16384 {
            return Err(CursorRejection::Malformed);
        }
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| CursorRejection::Malformed)?;
        let cursor: Self =
            serde_json::from_slice(&bytes).map_err(|_| CursorRejection::Malformed)?;
        if cursor.version != 2 {
            return Err(CursorRejection::Malformed);
        }
        cursor.binding.verify(context)?;
        Ok(cursor)
    }
}
