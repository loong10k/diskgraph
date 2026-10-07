//! v15 保留原归属与可审计拒绝，统一授权投影不暴露已隔离 revision。
use crate::{Result, StoreError};
use rusqlite::Connection;

const VIEW: &str = "CREATE VIEW revision_authorized_ownership AS SELECT o.revision_id,o.server_id,o.scope_id FROM revision_ownership o WHERE NOT EXISTS(SELECT 1 FROM revision_access_denials d WHERE d.revision_id=o.revision_id)";

/// 参数：原图库连接；返回：事务升级，保留全部快照和归属，不覆盖历史身份。
pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE revision_access_denials(revision_id TEXT PRIMARY KEY REFERENCES graph_revisions(revision_id) ON DELETE CASCADE,server_id TEXT NOT NULL,scope_id TEXT NOT NULL,reason TEXT NOT NULL CHECK(reason='root_identity_unconfirmed'));")?;
    connection.execute_batch(VIEW)?;
    connection.execute_batch("PRAGMA user_version=15; COMMIT;")?;
    Ok(())
}

/// 参数：原图库连接；返回：固定授权视图完整，损坏或替换不得启动新服务。
pub(crate) fn validate(connection: &Connection) -> Result<()> {
    let sql: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type='view' AND name='revision_authorized_ownership'",
        [],
        |row| row.get(0),
    )?;
    if sql != VIEW {
        return Err(StoreError::InvalidGraph(
            "revision authorization view differs from expected schema".into(),
        ));
    }
    connection.prepare(
        "SELECT revision_id,server_id,scope_id,reason FROM revision_access_denials LIMIT 0",
    )?;
    Ok(())
}
