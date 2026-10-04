use super::ProcessNativeSession;
use super::handle_reservation::HandleReservation;
use super::linux_open::{open_at, unique_mount};
use diskgraph_core::ProcessEvidenceFailureCode as Failure;
use std::ffi::CString;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

/// 原授权根的逐祖先名称租约；来源：Linux openat2/O_PATH，持有对象防止身份回收后重用。
/// 只比较绑定身份，不把相邻目录的 mtime/ctime 变化当作根替换。
pub(super) struct LinuxRootNamespace<'a> {
    anchor: File,
    route: Vec<(CString, File)>,
    _handles: HandleReservation<'a>,
}
impl<'a> LinuxRootNamespace<'a> {
    /// 参数：真实绝对 scope 根与原会话；返回：从 held `/` 开始的完整 no-follow 祖先链。
    pub(super) fn open(
        root: &Path,
        session: &'a ProcessNativeSession<'_>,
    ) -> Result<Self, Failure> {
        if !root.is_absolute()
            || root
                .components()
                .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        {
            return Err(Failure::Unsupported);
        }
        let count = root
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .count();
        let handles = u32::try_from(count)
            .ok()
            .and_then(|n| n.checked_add(2))
            .ok_or(Failure::BudgetExceeded)?;
        let reservation = session.reserve_handles(handles)?;
        let allocation = count
            .checked_mul(std::mem::size_of::<(CString, File)>())
            .and_then(|v| v.checked_add(root.as_os_str().len()))
            .and_then(|v| v.checked_add(count))
            .ok_or(Failure::BudgetExceeded)?;
        session.admit(512, 1, allocation as u64)?;
        let anchor = open_at(
            libc::AT_FDCWD,
            c"/",
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x04 | 0x02 | 0x20,
        )?;
        let mut route: Vec<(CString, File)> = Vec::new();
        route
            .try_reserve_exact(count)
            .map_err(|_| Failure::BudgetExceeded)?;
        for component in root.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            session.admit(512, 1, 0)?;
            let name = CString::new(name.as_bytes()).map_err(|_| Failure::Unsupported)?;
            let parent = route.last().map_or(&anchor, |(_, file)| file);
            // scope 根的合法祖先可跨挂载点；目标内部的 NO_XDEV 仍由 LinuxTarget 保持。
            let file = open_at(
                parent.as_raw_fd(),
                &name,
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0x08 | 0x04 | 0x02 | 0x20,
            )?;
            session.check()?;
            route.push((name, file));
        }
        Ok(Self {
            anchor,
            route,
            _handles: reservation,
        })
    }
    /// 参数：无；返回：原祖先链末端的 held scope 根，不重开源路径。
    pub(super) fn root(&self) -> &File {
        self.route.last().map_or(&self.anchor, |(_, file)| file)
    }
    /// 参数：同一原会话；返回：每个原 parent→name 仍绑定原对象，旁支活动不影响判断。
    pub(super) fn verify(&self, session: &ProcessNativeSession<'_>) -> Result<(), Failure> {
        let mut parent = &self.anchor;
        for (name, original) in &self.route {
            session.admit(1024, 1, 0)?;
            let rebound = open_at(
                parent.as_raw_fd(),
                name,
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0x08 | 0x04 | 0x02 | 0x20,
            )?;
            let before = original.metadata().map_err(|_| Failure::Unavailable)?;
            let after = rebound.metadata().map_err(|_| Failure::Unavailable)?;
            if before.dev() != after.dev()
                || before.ino() != after.ino()
                || unique_mount(original)? != unique_mount(&rebound)?
            {
                return Err(Failure::Conflict);
            }
            session.check()?;
            parent = original;
        }
        session.check()
    }
}
