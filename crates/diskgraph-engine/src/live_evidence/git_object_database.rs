//! D20 的有界私有扁平 ODB；只复制普通对象字节，不连接源目录或使用硬链接。

use super::git_directory_lease::GitDirectoryLease;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_metadata_tree::GitMetadataTree;
use super::git_private_directory::GitPrivateDirectory;
use super::probe_budget::ProbeBudget;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

/// 捕获源对象集合并复制到本次 owner 的唯一私有对象目录。
/// 参数：source/target 为原生源目录及私有目标，oid_len 为已验证格式的 OID 字节数；
/// metadata/budget/probe 为原视图的集合、累计额度及期限，owner 为已有私有容量所有者。
/// 返回：完整普通 loose/pack 副本；不支持的路径、外部对象来源和资源不足明确拒绝。
pub(super) fn copy_flat(
    source: &Path,
    target: &Path,
    oid_len: usize,
    metadata: &mut GitMetadataTree,
    owner: &mut GitPrivateDirectory,
    budget: &mut GitMetadataBudget,
    probe: &mut ProbeBudget,
) -> Result<(), String> {
    if !matches!(oid_len, 20 | 32) {
        return Err("unsupported Git object format".into());
    }
    if !metadata.directory(source, budget, probe)? {
        return Err("missing Git object directory".into());
    }
    // 名单及普通类型先通过；info 文件从不交给 Git，alternates 不读取原始内容。
    inspect_info(&source.join("info"), metadata, budget, probe)?;
    let names = metadata.names(source).to_vec();
    for name in names {
        budget.check(probe)?;
        match name.to_str() {
            Some("info" | "pack") => {}
            Some(name) if is_hex(name, 2) => {
                let fanout = source.join(name);
                if !metadata.directory(&fanout, budget, probe)? {
                    return Err("Git loose directory disappeared".into());
                }
                owner.create_dir_all(&target.join(name), probe)?;
                for leaf in metadata.names(&fanout).to_vec() {
                    budget.check(probe)?;
                    if !leaf
                        .to_str()
                        .is_some_and(|leaf| is_hex(leaf, oid_len * 2 - 2))
                    {
                        return Err("unsupported Git loose object name".into());
                    }
                    copy_file(
                        &fanout.join(&leaf),
                        &target.join(name).join(&leaf),
                        metadata,
                        owner,
                        budget,
                        probe,
                    )?;
                }
            }
            _ => return Err("unsupported Git object directory entry".into()),
        }
    }
    copy_packs(
        &source.join("pack"),
        &target.join("pack"),
        oid_len,
        metadata,
        owner,
        budget,
        probe,
    )?;
    budget.check(probe)
}

fn inspect_info(
    source: &Path,
    metadata: &mut GitMetadataTree,
    budget: &mut GitMetadataBudget,
    probe: &mut ProbeBudget,
) -> Result<(), String> {
    if !metadata.directory(source, budget, probe)? {
        return Ok(());
    }
    for name in metadata.names(source) {
        budget.check(probe)?;
        match name.to_str() {
            Some("alternates" | "http-alternates") => {
                return Err("unsupported Git object alternates".into());
            }
            Some("packs" | "commit-graph") => ordinary_type(&source.join(name), false, probe)?,
            Some("commit-graphs") => ordinary_type(&source.join(name), true, probe)?,
            _ => return Err("unsupported Git object info entry".into()),
        }
    }
    Ok(())
}

fn copy_packs(
    source: &Path,
    target: &Path,
    oid_len: usize,
    metadata: &mut GitMetadataTree,
    owner: &mut GitPrivateDirectory,
    budget: &mut GitMetadataBudget,
    probe: &mut ProbeBudget,
) -> Result<(), String> {
    if !metadata.directory(source, budget, probe)? {
        return Ok(());
    }
    let names = metadata.names(source).to_vec();
    let mut pairs = BTreeMap::<String, u8>::new();
    for name in &names {
        budget.check(probe)?;
        if name == OsStr::new("multi-pack-index") {
            ordinary_type(&source.join(name), false, probe)?;
            continue;
        }
        let (stem, extension) = name
            .to_str()
            .and_then(|name| name.rsplit_once('.'))
            .ok_or("unsupported Git pack name")?;
        if !stem
            .strip_prefix("pack-")
            .is_some_and(|oid| is_hex(oid, oid_len * 2))
        {
            return Err("unsupported Git pack name".into());
        }
        let flag = match extension {
            "pack" => 1,
            "idx" => 2,
            "promisor" => return Err("unsupported Git promisor object store".into()),
            "bitmap" | "rev" | "mtimes" | "keep" => {
                ordinary_type(&source.join(name), false, probe)?;
                continue;
            }
            _ => return Err("unsupported Git pack extension".into()),
        };
        *pairs.entry(stem.to_owned()).or_default() |= flag;
    }
    if pairs.values().any(|mask| *mask != 3) {
        return Err("unsupported Git unpaired pack/index".into());
    }
    owner.create_dir_all(target, probe)?;
    for stem in pairs.keys() {
        for extension in ["pack", "idx"] {
            budget.check(probe)?;
            let name = format!("{stem}.{extension}");
            copy_file(
                &source.join(&name),
                &target.join(name),
                metadata,
                owner,
                budget,
                probe,
            )?;
        }
    }
    Ok(())
}

fn copy_file(
    source: &Path,
    target: &Path,
    metadata: &mut GitMetadataTree,
    owner: &mut GitPrivateDirectory,
    budget: &mut GitMetadataBudget,
    probe: &mut ProbeBudget,
) -> Result<(), String> {
    let index = metadata
        .file(source, budget, probe)?
        .ok_or("Git object disappeared")?;
    // 沿用句柄捕获与完整末段重读；保留 Vec 消耗同一额度，不创建新的输入 owner。
    owner
        .write(
            target,
            metadata.get(index).bytes().expect("captured Git object"),
            probe,
        )
        .map_err(|error| format!("Git object copy: {error}"))
}

fn ordinary_type(path: &Path, directory: bool, probe: &mut ProbeBudget) -> Result<(), String> {
    probe.check().map_err(|error| error.to_string())?;
    // 忽略内容也不能通过父链接查询外部类型；租约仅在本次属性检查期间持有。
    if directory {
        GitDirectoryLease::open(path, probe)
            .map_err(|error| format!("unsupported Git object sidecar type: {error}"))?;
    } else {
        ordinary_file_type(path, probe)?;
    }
    probe.check().map_err(|error| error.to_string())
}

#[cfg(unix)]
fn ordinary_file_type(path: &Path, probe: &mut ProbeBudget) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let parent = path.parent().ok_or("invalid Git object sidecar parent")?;
    let lease = GitDirectoryLease::open(parent, probe)
        .map_err(|error| format!("unsupported Git object sidecar type: {error}"))?;
    let name = std::ffi::CString::new(
        path.file_name()
            .ok_or("invalid Git object sidecar name")?
            .as_bytes(),
    )
    .map_err(|_| "invalid Git object sidecar name")?;
    let mut state = std::mem::MaybeUninit::<libc::stat>::uninit();
    // 安全性：目录租约、单组件名称和输出缓冲存活至调用结束；只查属性，不打开内容。
    if unsafe {
        libc::fstatat(
            lease.leaf_file().as_raw_fd(),
            name.as_ptr(),
            state.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(format!(
            "Git object sidecar type: {}",
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { state.assume_init() }.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err("unsupported Git object sidecar type".into());
    }
    Ok(())
}

#[cfg(windows)]
fn ordinary_file_type(path: &Path, probe: &mut ProbeBudget) -> Result<(), String> {
    use crate::windows_scoped_file::WindowsScopedFile;
    let root = path.components().take(2).collect::<std::path::PathBuf>();
    // 属性租约逐组件禁止重解析；绝不调用 open_data，不读取被忽略文件的内容。
    let lease = WindowsScopedFile::open(&root, path)
        .map_err(|error| format!("unsupported Git object sidecar type: {error}"))?;
    if lease.state.placeholder() {
        return Err("unsupported Git object sidecar placeholder".into());
    }
    probe.check().map_err(|error| error.to_string())
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
