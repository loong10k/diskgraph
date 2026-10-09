//! 同一注册范围的借用准入投影；来源：Rust D42，不从显示定位重建原路径。
use crate::{ControlStore, Result, ScopeRecord, StoreError};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use diskgraph_core::{Locator, QueryReadBudget, ScopeId, ServerId};
use rusqlite::OptionalExtension;
use rusqlite::types::ValueRef;
use std::path::PathBuf;

pub(crate) const SCOPE_SQL: &str = "SELECT root_kind, root_raw_b64, root_display, volume_id, created_at_unix_ms, revoked, scope_id FROM scopes WHERE scope_id = ?1";

impl ControlStore {
    /// 按无损注册根窄读未撤销范围，不拥有显示文本或其他范围。来源：SC-02。
    /// 参数：root 为原始根定位，reads 为原请求账本；返回：实际范围身份或空值/预算错误。
    pub fn scope_id_for_root_with_budget(
        &self,
        root: &Locator,
        reads: &mut QueryReadBudget,
    ) -> Result<Option<ScopeId>> {
        if !reads.admit(0, 0, 0) {
            return Err(StoreError::BudgetExceeded);
        }
        let mut statement = self.connection.prepare(
            "SELECT scope_id FROM scopes WHERE root_kind=?1 AND root_raw_b64=?2 AND revoked=0",
        )?;
        let mut rows = statement.query(rusqlite::params![
            crate::control_codec::locator_kind_tag(root.kind),
            root.raw_b64
        ])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let ValueRef::Text(raw) = row.get_ref(0)? else {
            return Err(StoreError::InvalidGraph("scope id is not text".into()));
        };
        if !reads.admit(0, 0, raw.len()) {
            return Err(StoreError::BudgetExceeded);
        }
        let value =
            std::str::from_utf8(raw).map_err(|e| StoreError::InvalidGraph(e.to_string()))?;
        Ok(Some(
            ScopeId::new(value.to_owned()).map_err(|e| StoreError::InvalidGraph(e.to_string()))?,
        ))
    }

    /// 只查询范围的实时撤销标量，不读取显示文本或卷描述。来源：Rust SC-04/D42。
    /// 参数：实际范围身份；返回：撤销状态，缺失范围仍返回 ScopeNotFound。
    pub fn scope_revoked(&self, scope: &ScopeId) -> Result<bool> {
        self.connection
            .prepare_cached("SELECT revoked FROM scopes WHERE scope_id=?1")?
            .query_row([scope.as_str()], |row| Ok(row.get::<_, i64>(0)? != 0))
            .optional()?
            .ok_or_else(|| StoreError::ScopeNotFound(scope.as_str().into()))
    }

    /// 借用必要原始根字段并在解码前准入，不复制显示文本或卷描述。来源：Rust D42。
    /// 参数：实际范围及同一请求账本回调；返回：本平台无损原始路径，不访问文件系统。
    /// 撤销、非原生定位、格式错误及预算错误均拒绝，不以显示路径作为替代。
    pub fn scope_native_root_with_admission(
        &self,
        scope: &ScopeId,
        admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
    ) -> Result<PathBuf> {
        admit(0, 0, 0)?;
        let mut statement = self
            .connection
            .prepare("SELECT root_kind, root_raw_b64, revoked FROM scopes WHERE scope_id=?1")?;
        let mut rows = statement.query([scope.as_str()])?;
        let row = rows
            .next()?
            .ok_or_else(|| StoreError::ScopeNotFound(scope.as_str().into()))?;
        let fields = [row.get_ref(0)?, row.get_ref(1)?, row.get_ref(2)?];
        if row.get::<_, i64>(2)? != 0 {
            return Err(StoreError::Conflict("job scope revoked".into()));
        }
        if text(fields[0])? != "native_path" {
            return Err(StoreError::UnsupportedLocator(
                "scope root is not a native path".into(),
            ));
        }
        // SQLite ValueRef 仍借用连接数据；额度覆盖解码 Vec 与本平台路径转换后才分配。
        crate::metadata_read_cost::charge(
            admit,
            crate::metadata_read_cost::raw(&fields)?,
            0,
            std::mem::size_of::<PathBuf>(),
            1,
        )?;
        let raw = BASE64
            .decode(text(fields[1])?.as_bytes())
            .map_err(|_| StoreError::InvalidGraph("invalid scope root base64".into()))?;
        let path = native_path(raw)?;
        admit(0, 0, 0)?;
        Ok(path)
    }

    /// 参数：真实范围及原会话回调；返回：所有定位/卷/身份字段在拥有前累计准入的scope。
    pub fn scope_with_admission(
        &self,
        scope: &ScopeId,
        admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
    ) -> Result<ScopeRecord> {
        admit(0, 0, 0)?;
        let mut statement = self.connection.prepare(SCOPE_SQL)?;
        let mut rows = statement.query([scope.as_str()])?;
        let row = rows
            .next()?
            .ok_or_else(|| StoreError::ScopeNotFound(scope.as_str().into()))?;
        let fields = [
            row.get_ref(0)?,
            row.get_ref(1)?,
            row.get_ref(2)?,
            row.get_ref(3)?,
            row.get_ref(4)?,
            row.get_ref(5)?,
            row.get_ref(6)?,
        ];
        crate::metadata_read_cost::charge(
            admit,
            crate::metadata_read_cost::raw(&fields)?,
            0,
            std::mem::size_of::<ScopeRecord>(),
            1,
        )?;
        let invalid = || StoreError::InvalidGraph("invalid stored scope columns".into());
        let scope_id = ScopeId::new(text(fields[6])?).map_err(|_| invalid())?;
        let volume_id = match fields[3] {
            ValueRef::Null => None,
            value => Some(text(value)?.to_owned()),
        };
        let record = ScopeRecord {
            scope_id,
            root: Locator {
                kind: crate::control_codec::parse_locator_kind(text(fields[0])?),
                raw_b64: text(fields[1])?.to_owned(),
                display: text(fields[2])?.to_owned(),
            },
            volume_id,
            created_at_unix_ms: row.get::<_, i64>(4)?.try_into().unwrap_or(0),
            revoked: row.get::<_, i64>(5)? != 0,
        };
        admit(0, 0, 0)?;
        Ok(record)
    }

    /// 参数：原会话回调；返回：持久server字段拥有前准入并验证的本机身份，绝不创建身份。
    pub fn existing_server_id_with_admission(
        &self,
        admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
    ) -> Result<ServerId> {
        admit(0, 0, 0)?;
        let mut statement = self
            .connection
            .prepare("SELECT server_id FROM server WHERE id=1")?;
        let mut rows = statement.query([])?;
        let row = rows
            .next()?
            .ok_or_else(|| StoreError::InvalidGraph("stored server id is missing".into()))?;
        let field = row.get_ref(0)?;
        crate::metadata_read_cost::charge(
            admit,
            crate::metadata_read_cost::raw(&[field])?,
            0,
            std::mem::size_of::<ServerId>(),
            1,
        )?;
        let id = ServerId::new(
            field
                .as_str()
                .map_err(|_| StoreError::InvalidGraph("invalid server id column".into()))?,
        )
        .map_err(|_| StoreError::InvalidGraph("invalid stored server id".into()))?;
        admit(0, 0, 0)?;
        Ok(id)
    }
}

fn text<'a>(value: ValueRef<'a>) -> Result<&'a str> {
    value
        .as_str()
        .map_err(|_| StoreError::InvalidGraph("invalid stored scope columns".into()))
}

fn native_path(raw: Vec<u8>) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        if raw.is_empty() || raw.contains(&0) {
            return Err(StoreError::InvalidGraph("invalid scope native path".into()));
        }
        Ok(std::ffi::OsString::from_vec(raw).into())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let (pairs, remainder) = raw.as_chunks::<2>();
        if pairs.is_empty() || !remainder.is_empty() {
            return Err(StoreError::InvalidGraph(
                "invalid scope native encoding".into(),
            ));
        }
        let units: Vec<u16> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        if units.contains(&0) {
            return Err(StoreError::InvalidGraph("invalid scope native path".into()));
        }
        Ok(std::ffi::OsString::from_wide(&units).into())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = raw;
        Err(StoreError::UnsupportedLocator(
            "scope native encoding is unavailable on this platform".into(),
        ))
    }
}
