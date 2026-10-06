use super::bounded_open_retry::bounded_open_retry;
use super::linux_directory_identity::LinuxDirectoryIdentity;
use super::linux_open::{error_code, open_at_io};
use diskgraph_core::ProcessEvidenceFailureCode as Failure;
use std::ffi::CString;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

/// 扫描起点的原生锚和逐组件名称租约；来源：Rust FS-02 / Linux openat2、statx。
/// 仅保留一条根链，沿用扫描原检查，不创建 Process 会话或按节点累积目录句柄。
pub(super) struct LinuxScanNamespace {
    anchor: LinuxDirectoryIdentity,
    route: Vec<(CString, LinuxDirectoryIdentity)>,
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
        let mut route: Vec<(CString, LinuxDirectoryIdentity)> = Vec::new();
        route
            .try_reserve_exact(count)
            .map_err(|_| Failure::BudgetExceeded)?;
        check()?;
        let anchor = reopen_for_verification(
            libc::AT_FDCWD,
            c"/",
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x04 | 0x02 | 0x20,
            check,
        )?;
        check()?;
        // 先区分内核缺少 unique-mount 能力；取得原绑定之后的复核失败不能退成 capability gap。
        let anchor = LinuxDirectoryIdentity::capture(anchor, check)?;
        check()?;
        for component in path.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            check()?;
            let name = CString::new(name.as_bytes()).map_err(|_| Failure::Unsupported)?;
            let parent = route
                .last()
                .map_or(anchor.file(), |(_, identity)| identity.file());
            // 合法注册根的祖先可以跨挂载；每个捕获对象的唯一挂载 ID 在复核时单独比较。
            let file = reopen_for_verification(
                parent.as_raw_fd(),
                &name,
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0x08 | 0x04 | 0x02 | 0x20,
                check,
            )?;
            check()?;
            // 原 FD 的身份只捕获一次；构造末尾仍从当前 / 完整复核，避免捕获期间重绑定。
            let identity =
                LinuxDirectoryIdentity::capture(file, check).map_err(|error| match error {
                    Failure::BudgetExceeded => Failure::BudgetExceeded,
                    _ => Failure::Conflict,
                })?;
            route.push((name, identity));
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
        self.route
            .last()
            .map_or(self.anchor.file(), |(_, identity)| identity.file())
    }
    /// 参数：同一原扫描检查；返回：当前锚和每个原 parent→name 仍绑定原对象或固定失败。
    /// 只比较 dev/inode/唯一挂载，不比较目录时间；允许无关 sibling 活动，非原子 namespace 快照。
    pub(super) fn verify(&self, check: &dyn Fn() -> Result<(), Failure>) -> Result<(), Failure> {
        check()?;
        let current_anchor = reopen_for_verification(
            libc::AT_FDCWD,
            c"/",
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x04 | 0x02 | 0x20,
            check,
        )?;
        self.anchor.verify_current(&current_anchor, check)?;
        let mut parent = current_anchor;
        for (name, original) in &self.route {
            check()?;
            let current = reopen_for_verification(
                parent.as_raw_fd(),
                name,
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0x08 | 0x04 | 0x02 | 0x20,
                check,
            )?;
            original.verify_current(&current, check)?;
            parent = current;
        }
        check()
    }
}

// 仅重试 openat2 的临时竞态；原解析标志、原检查和逐组件身份复核保持不变。
fn reopen_for_verification(
    parent: i32,
    name: &std::ffi::CStr,
    flags: i32,
    resolve: u64,
    check: &dyn Fn() -> Result<(), Failure>,
) -> Result<File, Failure> {
    bounded_open_retry(check, &mut || open_at_io(parent, name, flags, resolve), libc::EAGAIN)?
        .map_err(|error| {
            let errno = error.raw_os_error();
            // 持续临时竞态不是能力缺失；没有成功复核的名称绑定绝不能发布。
            let failure = if errno == Some(libc::EAGAIN) {
                Failure::Conflict
            } else {
                error_code(errno)
            };
            eprintln!("diskgraph: native scan namespace reopen failed: class={failure:?}; errno={errno:?}; flags={flags}; resolve={resolve}");
            failure
        })
}
