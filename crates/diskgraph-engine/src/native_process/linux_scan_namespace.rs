use super::linux_open::{open_at, unique_mount};
use diskgraph_core::ProcessEvidenceFailureCode as Failure;
use std::ffi::CString;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

/// 扫描起点的原生锚和逐组件名称租约；来源：Rust FS-02 / Linux openat2、statx。
/// 仅保留一条根链，沿用扫描原检查，不创建 Process 会话或按节点累积目录句柄。
pub(super) struct LinuxScanNamespace {
    anchor: File,
    route: Vec<(CString, File)>,
}
impl LinuxScanNamespace {
    /// 参数：原始注册根和原扫描检查；返回：保留全部原祖先绑定的有限租约或原生失败。
    pub(super) fn open(
        path: &Path,
        check: &dyn Fn() -> Result<(), Failure>,
    ) -> Result<Self, Failure> {
        check()?;
        // 保留此前完整绝对路径 open 的长度边界，不能通过拆组件放大可接收路径和 FD 成本。
        if !path.is_absolute()
            || path.as_os_str().len() >= libc::PATH_MAX as usize
            || path
                .components()
                .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        {
            return Err(Failure::Unsupported);
        }
        let count = path
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .count();
        let mut route: Vec<(CString, File)> = Vec::new();
        route
            .try_reserve_exact(count)
            .map_err(|_| Failure::BudgetExceeded)?;
        check()?;
        let anchor = open_at(
            libc::AT_FDCWD,
            c"/",
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x04 | 0x02 | 0x20,
        )?;
        check()?;
        // 先区分内核缺少 unique-mount 能力；取得原绑定之后的复核失败不能退成 capability gap。
        unique_mount(&anchor)?;
        check()?;
        for component in path.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            check()?;
            let name = CString::new(name.as_bytes()).map_err(|_| Failure::Unsupported)?;
            let parent = route.last().map_or(&anchor, |(_, file)| file);
            // 合法注册根的祖先可以跨挂载；每个捕获对象的唯一挂载 ID 在复核时单独比较。
            let file = open_at(
                parent.as_raw_fd(),
                &name,
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0x08 | 0x04 | 0x02 | 0x20,
            )?;
            check()?;
            route.push((name, file));
        }
        let value = Self { anchor, route };
        value.verify(check).map_err(|error| match error {
            Failure::BudgetExceeded => Failure::BudgetExceeded,
            _ => Failure::Conflict,
        })?;
        Ok(value)
    }
    /// 参数：无；返回：末端原根句柄的借用，不按显示路径重新定位文件。
    pub(super) fn root(&self) -> &File {
        self.route.last().map_or(&self.anchor, |(_, file)| file)
    }
    /// 参数：同一原扫描检查；返回：当前锚和每个原 parent→name 仍绑定原对象或固定失败。
    /// 只比较 dev/inode/唯一挂载，不比较目录时间；允许无关 sibling 活动，非原子 namespace 快照。
    pub(super) fn verify(&self, check: &dyn Fn() -> Result<(), Failure>) -> Result<(), Failure> {
        check()?;
        let current_anchor = open_at(
            libc::AT_FDCWD,
            c"/",
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x04 | 0x02 | 0x20,
        )?;
        same_identity(&self.anchor, &current_anchor, check)?;
        let mut parent = current_anchor;
        for (name, original) in &self.route {
            check()?;
            let current = open_at(
                parent.as_raw_fd(),
                name,
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0x08 | 0x04 | 0x02 | 0x20,
            )?;
            same_identity(original, &current, check)?;
            parent = current;
        }
        check()
    }
}

fn same_identity(
    original: &File,
    current: &File,
    check: &dyn Fn() -> Result<(), Failure>,
) -> Result<(), Failure> {
    check()?;
    let before = original.metadata().map_err(|_| Failure::Unavailable)?;
    check()?;
    let after = current.metadata().map_err(|_| Failure::Unavailable)?;
    check()?;
    let original_mount = unique_mount(original)?;
    check()?;
    let current_mount = unique_mount(current)?;
    check()?;
    if before.dev() != after.dev() || before.ino() != after.ino() || original_mount != current_mount
    {
        return Err(Failure::Conflict);
    }
    Ok(())
}
