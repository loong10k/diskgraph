//! 按快照与节点主键读取定位；所有借用字段在拥有和解码前进入共享预算。

use crate::node_codec::as_i64;
use crate::{Result, SqliteSnapshotStore, StoreError, StoredNodeLocator};
use diskgraph_core::{
    LocatorEncoding, LocatorKind, QualifiedLocator, QueryReadBudget, ResourceLocator,
};
use rusqlite::{params, types::ValueRef};

impl SqliteSnapshotStore {
    /// 精确读取一个节点的可用原生定位，绝不从显示文本构造原始路径。
    /// 参数：固定 snapshot_id/node_id 和调用者共享的节点、字节、绝对期限预算。
    /// 返回：节点缺失为 None，旧节点的 locator=None；损坏、不支持编码或超预算明确报错。
    pub fn native_locator_bounded(
        &self,
        snapshot_id: &str,
        node_id: u64,
        budget: &mut QueryReadBudget,
    ) -> Result<Option<StoredNodeLocator>> {
        check(budget)?;
        let mut statement = self.connection.prepare("SELECT native_locator_kind,native_locator_encoding,native_locator_raw,locator_key,self_modified_unix_seconds FROM nodes WHERE snapshot_id=?1 AND id=?2")?;
        let mut rows = statement.query(params![snapshot_id, as_i64(node_id)?])?;
        let Some(row) = rows.next()? else {
            check(budget)?;
            return Ok(None);
        };
        let fields = [
            row.get_ref(0)?,
            row.get_ref(1)?,
            row.get_ref(2)?,
            row.get_ref(3)?,
            row.get_ref(4)?,
        ];
        let bytes = fields.iter().try_fold(0usize, |size, field| {
            let len = match field {
                ValueRef::Null => 0,
                ValueRef::Integer(_) | ValueRef::Real(_) => 8,
                ValueRef::Text(value) | ValueRef::Blob(value) => value.len(),
            };
            size.checked_add(len).ok_or(StoreError::BudgetExceeded)
        })?;
        if !budget.admit(1, 0, bytes) {
            return Err(StoreError::BudgetExceeded);
        }
        let self_modified = match fields[4] {
            ValueRef::Null => None,
            ValueRef::Integer(value) => Some(value),
            _ => return Err(invalid("invalid self modification timestamp column")),
        };
        let locator = match (fields[0], fields[1], fields[2]) {
            (ValueRef::Null, ValueRef::Null, ValueRef::Null) => None,
            (ValueRef::Text(kind), ValueRef::Text(encoding), ValueRef::Blob(raw)) => {
                let kind = match kind {
                    b"native_path" => LocatorKind::NativePath,
                    b"document_uri" => LocatorKind::DocumentUri,
                    _ => return Err(invalid("unknown locator kind")),
                };
                let encoding = std::str::from_utf8(encoding)
                    .map_err(|_| invalid("locator encoding label is not UTF-8"))?;
                let encoding = LocatorEncoding::parse(encoding)
                    .map_err(|error| invalid(&format!("invalid locator encoding: {error}")))?;
                let display = match fields[3] {
                    ValueRef::Text(value) => std::str::from_utf8(value)
                        .map_err(|_| invalid("locator display is not UTF-8"))?,
                    _ => return Err(invalid("locator display column is not TEXT")),
                };
                let v1: ResourceLocator = serde_json::from_str(display)?;
                let display = match (kind, v1) {
                    (LocatorKind::NativePath, ResourceLocator::NativePath(display))
                    | (LocatorKind::DocumentUri, ResourceLocator::DocumentUri(display)) => display,
                    _ => return Err(invalid("locator kind disagrees with display record")),
                };
                let locator =
                    QualifiedLocator::from_parts(kind, encoding, raw.to_vec(), display)
                        .map_err(|error| invalid(&format!("invalid stored locator: {error}")))?;
                // 只接受本宿主可执行的原生定位；URI/异平台编码不得回退到 display。
                locator
                    .validate_native_path()
                    .map_err(|error| StoreError::UnsupportedLocator(error.to_string()))?;
                Some(locator)
            }
            _ => return Err(invalid("partial or invalid locator columns")),
        };
        check(budget)?;
        Ok(Some(StoredNodeLocator {
            locator,
            self_modified,
        }))
    }
}

fn check(budget: &mut QueryReadBudget) -> Result<()> {
    if budget.check() {
        Ok(())
    } else {
        Err(StoreError::BudgetExceeded)
    }
}
fn invalid(reason: &str) -> StoreError {
    StoreError::InvalidGraph(reason.into())
}
