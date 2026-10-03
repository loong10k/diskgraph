//! cargo_inventory：既有文件操作职责的原生 Rust 实现。
use crate::OpsError;
use crate::specialist::adapter_capability::CARGO_CLEAN;
use crate::specialist::cleanup_inventory::CleanupInventory;
use crate::specialist::inventory_object::InventoryObject;
use std::path::Path;

/// 测量目录树当前可见字节。
/// 参数：path 为测量目录。
/// 返回：原有递归累计结果，不提供原子快照或严格遍历预算。
/// Sums the bytes of a directory tree; measurement, not a promise.
pub(super) fn dir_size(path: &Path) -> u64 {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return 0,
    };
    if !metadata.is_dir() {
        return metadata.len();
    }
    let mut total = 0_u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return total;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total = total.saturating_add(dir_size(&path));
        } else if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            total = total.saturating_add(metadata.len());
        }
    }
    total
}

/// 以默认环境观察 Cargo 清理对象。
/// 参数：project_dir 为项目目录；目标目录覆盖读取当前 CARGO_TARGET_DIR。
/// 返回：目标对象和活动构建清单或非项目错误；不运行 Cargo 子进程。
/// Inventories the Cargo build output of one project directory, reading the
/// real environment for the shared-target setting.
pub fn cargo_inventory(project_dir: &Path) -> Result<CleanupInventory, OpsError> {
    let shared = std::env::var("CARGO_TARGET_DIR").ok();
    cargo_inventory_with_env(project_dir, shared.as_deref())
}

/// 以指定环境观察 Cargo 输出目录。
/// 参数：project_dir 为项目目录；cargo_target_dir 为可选目标目录覆盖。
/// 返回：对象、共享目录提示和锁占用观察，或非项目错误。
/// The inventory with the environment injected, so tests never mutate process
/// state to exercise a setting.
///
/// Without running cargo (EC-03), the target directory's existence and cargo's
/// own lock markers answer everything the inventory needs. The real target
/// scope is the project-local `target/`. A directory named by
/// `CARGO_TARGET_DIR` lives outside the project, is potentially shared with
/// other projects, and is therefore reported as a note but never offered as
/// an object. A `.cargo-lock` inside the target tree marks an active build,
/// and the inventory says cleanup must wait (EC-03, OP-04).
pub fn cargo_inventory_with_env(
    project_dir: &Path,
    cargo_target_dir: Option<&str>,
) -> Result<CleanupInventory, OpsError> {
    if !project_dir.join("Cargo.toml").is_file() {
        return Err(OpsError::Stale(
            "the project directory has no Cargo.toml; it is not a cargo project".into(),
        ));
    }
    let mut notes = Vec::new();
    let mut objects = Vec::new();
    let mut active_build = false;
    let local = project_dir.join("target");
    if std::fs::symlink_metadata(&local).is_ok_and(|metadata| metadata.is_dir()) {
        // The lock FILE persists after a build finishes (verified on a real
        // host), so existence proves nothing. Cargo guards a build with an
        // advisory lock on that file, so an uncontended lock attempt is the
        // honest active-build signal.
        let lock = local.join("debug").join(".cargo-lock");
        if lock.is_file() && build_lock_is_contended(&lock) {
            active_build = true;
            notes.push(
                "cargo's build lock is held: a build is in progress; cleanup must wait".into(),
            );
        }
        objects.push(InventoryObject {
            path: local.clone(),
            bytes: dir_size(&local),
            kind: "cargo-target",
        });
    } else {
        notes.push("the project has no target directory; there is nothing to clean".into());
    }
    if let Some(shared) = cargo_target_dir.filter(|shared| !shared.is_empty()) {
        notes.push(format!(
            "CARGO_TARGET_DIR={shared} names a build directory outside this project; \
             it may be shared with other projects and is never planned"
        ));
    }
    Ok(CleanupInventory {
        adapter: CARGO_CLEAN.id,
        objects,
        notes,
        active_build,
    })
}

/// 探测 Cargo 构建锁占用。
/// 参数：lock 为目标目录中的锁文件；既有实现可能创建缺失的锁文件。
/// 返回：原有平台锁探测结果。
/// True when some process already holds an advisory lock on `lock`. The
/// attempt never blocks: acquiring and immediately releasing the lock is the
/// harmless probe; contention is the evidence.
pub(super) fn build_lock_is_contended(lock: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock)
        {
            Ok(file) => file,
            Err(_) => return false,
        };
        // SAFETY: flock on a file we just opened is the documented advisory
        // protocol; the descriptors are valid for the duration of the calls.
        let acquired = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if acquired == 0 {
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
            return false;
        }
        true
    }
    #[cfg(not(unix))]
    {
        let _ = lock;
        false
    }
}
