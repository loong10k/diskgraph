//! path_revalidation：既有文件操作职责的原生 Rust 实现。
use crate::path_fault::PathFault;
use crate::side::Side;
use std::path::Path;

/// 逐组件复核可信根以下的路径。
/// 参数：trusted_root 为可信根；path 为目标路径；side 指定源端或目标端。
/// 返回：通过为 Ok；链接、越界或消失返回 PathFault。
/// Revalidates the components of `path` that live **below** `trusted_root`,
/// without following links.
///
/// Only the managed subtree is checked. System prefixes are deliberately
/// trusted: on macOS `/var` is itself a symlink into `/private/var`, so a naive
/// walk from the filesystem root would refuse every legitimate operation
/// (OP-04, scoped to what the deployment actually manages).
pub fn revalidate_below(trusted_root: &Path, path: &Path, side: Side) -> Result<(), PathFault> {
    let relative = path
        .strip_prefix(trusted_root)
        .map_err(|_| PathFault::ComponentVanished)?;
    // Walk the real components under the root, resolving each one without
    // following links, so a link planted mid-path is seen as a link.
    let mut current = trusted_root.to_path_buf();
    for component in relative.components() {
        use std::path::Component;
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(PathFault::ComponentVanished);
            }
            Component::Normal(name) => {
                current.push(name);
                match std::fs::symlink_metadata(&current) {
                    Ok(metadata) => {
                        if metadata.file_type().is_symlink() {
                            return Err(match side {
                                Side::Source => PathFault::SourceIsLink,
                                Side::Target => PathFault::TargetIsLink,
                            });
                        }
                    }
                    // A destination that does not exist yet is normal; a
                    // source that does not is not.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        if matches!(side, Side::Source) {
                            return Err(PathFault::ComponentVanished);
                        }
                    }
                    Err(_) => return Err(PathFault::ComponentVanished),
                }
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Ok(())
}
