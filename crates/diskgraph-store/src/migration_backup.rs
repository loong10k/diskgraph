//! 一致性迁移备份：独占预留新文件，绝不覆盖已有恢复材料。

use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

/// 参数：connection 为待迁移的真实 SQLite 连接，preferred 为兼容的首选备份名。
/// 返回：完成一致性备份的新路径；命名冲突时使用新名称，其他 I/O 错误立即传播。
/// 失败不返回可用备份；预留的空文件可能保留，但不得覆盖此前的恢复副本。
pub(crate) fn create_migration_backup(
    connection: &Connection,
    preferred: &Path,
) -> crate::Result<PathBuf> {
    let mut candidate = preferred.to_path_buf();
    for _ in 0..16 {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(reservation) => {
                // create_new 原子排除已有文件、链接和并发预留，首次备份名称保持兼容。
                drop(reservation);
                connection.backup(rusqlite::MAIN_DB, &candidate, None)?;
                return Ok(candidate);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let mut name = preferred.as_os_str().to_os_string();
                name.push(format!(".{}.bak", uuid::Uuid::new_v4()));
                candidate = PathBuf::from(name);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(std::io::Error::new(
        ErrorKind::AlreadyExists,
        "migration backup reservation exhausted",
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::create_migration_backup;
    use rusqlite::Connection;
    use std::sync::{Arc, Barrier};

    #[test]
    fn concurrent_migration_backups_keep_both_original_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let preferred = directory.path().join("database.pre-upgrade.bak");
        let barrier = Arc::new(Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|value| {
                let preferred = preferred.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let connection = Connection::open_in_memory().unwrap();
                    connection
                        .execute_batch("CREATE TABLE recovery_marker(value INTEGER);")
                        .unwrap();
                    connection
                        .execute("INSERT INTO recovery_marker VALUES (?1)", [value])
                        .unwrap();
                    barrier.wait();
                    (
                        create_migration_backup(&connection, &preferred).unwrap(),
                        value,
                    )
                })
            })
            .collect();
        let backups: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_ne!(backups[0].0, backups[1].0);
        for (path, expected) in backups {
            let value: i64 = Connection::open(path)
                .unwrap()
                .query_row("SELECT value FROM recovery_marker", [], |row| row.get(0))
                .unwrap();
            assert_eq!(value, expected);
        }
    }
}
