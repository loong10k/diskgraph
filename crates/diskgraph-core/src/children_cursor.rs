use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::{CursorContext, CursorRejection, PagingCursor};

/// v2 目录游标：绑定授权/过滤上下文及尺寸降序、名称/ID 升序的实际位置。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildrenCursor {
    pub version: u8,
    pub binding: PagingCursor,
    pub last_bytes: u64,
    pub last_name: String,
    pub last_id: u64,
}

impl ChildrenCursor {
    /// 编码游标；调用方仍需把编码后的游标计入响应预算。
    pub fn encode(&self) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(self).expect("cursor fields serialize"))
    }

    /// 校验参数为编码游标与当前授权上下文；返回有效 keyset，过期/旧版/越界拒绝。
    pub fn decode(encoded: &str, context: &CursorContext<'_>) -> Result<Self, CursorRejection> {
        if encoded.len() > 16_384 {
            return Err(CursorRejection::Malformed);
        }
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| CursorRejection::Malformed)?;
        let cursor: Self =
            serde_json::from_slice(&bytes).map_err(|_| CursorRejection::Malformed)?;
        if cursor.version != 2
            || cursor.last_bytes > i64::MAX as u64
            || cursor.last_id == 0
            || cursor.last_id > i64::MAX as u64
        {
            return Err(CursorRejection::Malformed);
        }
        cursor.binding.verify(context)?;
        Ok(cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::ChildrenCursor;
    use crate::{CursorContext, PagingCursor};

    #[test]
    fn directory_cursor_binds_every_query_dimension_and_refuses_legacy() {
        let context = CursorContext {
            principal_binding: "alice",
            scope_id: "scope",
            revision_id: "rev",
            filter_binding: "parent:1,min_bytes:0",
            sort_binding: "subtree_bytes_desc,name_asc,id_asc,children_keyset_v2",
            policy_version: 1,
        };
        let cursor = ChildrenCursor {
            version: 2,
            binding: PagingCursor::issue(&context, context.filter_binding, context.sort_binding, 4),
            last_bytes: 4096,
            last_name: "文件-é-ß".into(),
            last_id: 5,
        };
        let encoded = cursor.encode();
        assert_eq!(ChildrenCursor::decode(&encoded, &context).unwrap(), cursor);
        for changed in [
            CursorContext {
                principal_binding: "bob",
                ..context.clone()
            },
            CursorContext {
                scope_id: "other",
                ..context.clone()
            },
            CursorContext {
                revision_id: "new",
                ..context.clone()
            },
            CursorContext {
                filter_binding: "parent:2,min_bytes:0",
                ..context.clone()
            },
            CursorContext {
                sort_binding: "name_asc",
                ..context.clone()
            },
            CursorContext {
                policy_version: 2,
                ..context.clone()
            },
        ] {
            assert!(ChildrenCursor::decode(&encoded, &changed).is_err());
        }
        assert!(ChildrenCursor::decode(&cursor.binding.encode(), &context).is_err());
        for (version, bytes, id) in [
            (1, 4096, 5),
            (2, u64::MAX, 5),
            (2, 4096, u64::MAX),
            (2, 4096, 0),
        ] {
            let invalid = ChildrenCursor {
                version,
                last_bytes: bytes,
                last_id: id,
                ..cursor.clone()
            };
            assert!(ChildrenCursor::decode(&invalid.encode(), &context).is_err());
        }
        assert!(ChildrenCursor::decode(&"a".repeat(16_385), &context).is_err());
    }
}
