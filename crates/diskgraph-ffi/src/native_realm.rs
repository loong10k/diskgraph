//! 本机数据库 realm 解析、旧库归属检查与引擎初始化。
use diskgraph_core::ResourceLocator;
use diskgraph_store::SqliteSnapshotStore;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 构造绑定所使用的可信本地主体标识。
/// 参数：无输入。
/// 返回：固定 local-user 标识或格式校验错误。
pub(crate) fn local_principal() -> Result<diskgraph_core::PrincipalId, String> {
    diskgraph_core::PrincipalId::new("local-user").map_err(|error| error.to_string())
}

/// Stable on-disk realm for one graph file; sibling graphs never share jobs.
/// 为一个图库路径计算独立持久 realm 目录。
/// 参数：database 为待规范化的图库路径。
/// 返回：同级隐藏 realm 下按原始路径摘要隔离的目录，或路径错误。
pub(crate) fn realm_dir_for_database(database: &Path) -> Result<PathBuf, String> {
    let database = canonical_database_path(database)?;
    let parent = database.parent().ok_or("graph database has no parent")?;
    Ok(parent
        .join(".diskgraph-realms")
        .join(path_digest(&database)))
}

/// 按宿主原始路径编码计算稳定摘要。
/// 参数：path 为数据库规范路径。
/// 返回：SHA-256 十六进制字符串。
pub(crate) fn path_digest(path: &Path) -> String {
    let mut hasher = Sha256::new();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(path.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in path.as_os_str().encode_wide() {
            hasher.update(unit.to_le_bytes());
        }
    }
    #[cfg(not(any(unix, windows)))]
    hasher.update(path.to_string_lossy().as_bytes());
    hex::encode(hasher.finalize())
}

fn canonical_database_path(database: &Path) -> Result<PathBuf, String> {
    let parent = database
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let parent = parent.canonicalize().map_err(|error| error.to_string())?;
    let name = database
        .file_name()
        .ok_or("graph database needs a file name")?;
    let path = parent.join(name);
    if path.exists() {
        path.canonicalize().map_err(|error| error.to_string())
    } else {
        Ok(path)
    }
}

/// An existing shared control database is reused only if every registered
/// scope resolves to a published revision in this graph. Ambiguous history
/// stays unbound and must be reindexed by an administrator.
fn legacy_realm_matches(database: &Path, legacy: &Path) -> Result<bool, String> {
    if !database.is_file() || !legacy.is_file() {
        return Ok(false);
    }
    let mut control =
        diskgraph_store::ControlStore::open(legacy).map_err(|error| error.to_string())?;
    let scopes = control.list_scopes().map_err(|error| error.to_string())?;
    if scopes.is_empty() {
        return Ok(false);
    }
    let (graph, _) = SqliteSnapshotStore::open_with_backup(
        database,
        &legacy
            .parent()
            .unwrap_or(Path::new("."))
            .join("migration_backups"),
    )
    .map_err(|error| error.to_string())?;
    let server = control.ensure_server().map_err(|error| error.to_string())?;
    for scope in scopes {
        let locator = match scope.root.kind {
            diskgraph_core::LocatorKind::NativePath => {
                ResourceLocator::NativePath(scope.root.display().to_owned())
            }
            diskgraph_core::LocatorKind::DocumentUri => {
                ResourceLocator::DocumentUri(scope.root.display().to_owned())
            }
        };
        let Some(revision) = graph
            .latest_revision_for_root(&locator)
            .map_err(|error| error.to_string())?
        else {
            return Ok(false);
        };
        if let Some((owner_server, owner_scope)) = graph
            .revision_ownership(&revision)
            .map_err(|error| error.to_string())?
            && (owner_server != server.as_str() || owner_scope != scope.scope_id.as_str())
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn realm_for_database(database: &Path) -> Result<PathBuf, String> {
    let database = canonical_database_path(database)?;
    let parent = database.parent().ok_or("graph database has no parent")?;
    let legacy = parent.join("diskgraph-control.sqlite");
    let marker = parent.join(".diskgraph-legacy-graph");
    let digest = path_digest(&database);
    if let Ok(bound) = std::fs::read_to_string(&marker) {
        if bound.trim() == digest && legacy_realm_matches(&database, &legacy)? {
            return Ok(parent.to_path_buf());
        }
    } else if legacy_realm_matches(&database, &legacy)? {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(mut file) => {
                file.write_all(digest.as_bytes())
                    .map_err(|error| error.to_string())?;
                file.sync_all().map_err(|error| error.to_string())?;
                return Ok(parent.to_path_buf());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::fs::read_to_string(&marker)
                    .map_err(|error| error.to_string())?
                    .trim()
                    == digest
                {
                    return Ok(parent.to_path_buf());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    realm_dir_for_database(&database)
}

/// 解析持久 realm 并打开绑定共享的本地 Engine。
/// 参数：path 为图库路径。
/// 返回：已引导本地管理员的 Engine，或打开与授权错误。
pub(crate) fn open_engine(path: &str) -> Result<diskgraph_engine::Engine, String> {
    let database = canonical_database_path(Path::new(path))?;
    let realm = realm_for_database(&database)?;
    let engine = diskgraph_engine::Engine::open(diskgraph_engine::EngineConfig {
        data_dir: realm,
        graph_database_path: Some(database),
        ..Default::default()
    })
    .map_err(|error| error.to_string())?;
    engine
        .bootstrap_local_admin(&local_principal()?)
        .map_err(|error| error.to_string())?;
    Ok(engine)
}
