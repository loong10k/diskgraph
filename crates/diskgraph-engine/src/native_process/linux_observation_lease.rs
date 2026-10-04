use super::ProcessNativeSession;
use super::handle_reservation::HandleReservation;
use super::linux_directory::LinuxDirectory;
use super::linux_metadata::now_ms;
use super::linux_open::open_at;
use super::linux_pid::LinuxPid;
use super::linux_proc_root::LinuxProcRoot;
use super::linux_target::LinuxTarget;
use diskgraph_core::{
    IndexedFileEpoch, ProcessEvidenceFailureCode as Failure, ProcessEvidenceSummary,
    ProcessObservationCode as Code, ProcessObservationCoverage, ProcessObservationMethod,
    ProcessStartupIdentity,
};
use std::os::fd::AsRawFd;
use std::path::Path;

/// 跨观察、编码和发布前复核的原生资源租约；来源：Rust D42，不拥有任务或独立预算。
/// File 自身负责关闭；借用 reservation 仅保留同会话句柄峰值，不声称文件系统与 SQL 原子提交。
pub(crate) struct LinuxObservationLease<'a, 'session> {
    procfs: LinuxProcRoot,
    target: LinuxTarget<'a>,
    expected: &'a IndexedFileEpoch,
    session: &'a ProcessNativeSession<'session>,
    _handles: HandleReservation<'a>,
}
impl<'a, 'session> LinuxObservationLease<'a, 'session> {
    /// 参数：实际 scope、相对资源、原扫描 epoch 与同账本；返回：完整祖先及目标 held 租约。
    pub(crate) fn open(
        root: &Path,
        relative: &Path,
        expected: &'a IndexedFileEpoch,
        session: &'a ProcessNativeSession<'session>,
    ) -> Result<Self, Failure> {
        let result = (|| {
            // proc/self/枚举/PID/fd 临时句柄；祖先、目标与重绑定临时句柄由目标自己的 reservation 计数。
            let handles = session.reserve_handles(10)?;
            session.admit(8192, 1, 0)?;
            let procfs = LinuxProcRoot::open(&|| session.check())?;
            let target = LinuxTarget::open(root, relative, expected, &procfs.boot, session)?;
            Ok(Self {
                procfs,
                target,
                expected,
                session,
                _handles: handles,
            })
        })();
        match result {
            Ok(value) => Ok(value),
            Err(error) => session.fail(error),
        }
    }
    /// 参数：无；返回：逐资源启动身份与明确 Partial 覆盖；失败沿原会话锁存。
    pub(crate) fn observe(&self) -> Result<ProcessEvidenceSummary, Failure> {
        let result = (|| {
            let start = now_ms()?;
            self.session.check()?;
            let own = open_at(
                self.procfs.file.as_raw_fd(),
                c"self",
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0x08 | 0x01,
            )?;
            let own_identity = own.metadata().map_err(|_| Failure::Unavailable)?;
            self.session.check()?;
            let enumeration = open_at(
                self.procfs.file.as_raw_fd(),
                c".",
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0x08 | 0x04 | 0x02 | 0x01,
            )?;
            let mut entries = LinuxDirectory::new(enumeration, self.session)?;
            let mut processes = Vec::new();
            let mut codes = Vec::new();
            self.session
                .admit(0, 0, 9 * std::mem::size_of::<Code>() as u64)?;
            codes
                .try_reserve_exact(9)
                .map_err(|_| Failure::BudgetExceeded)?;
            codes.push(Code::VisibilityRestricted);
            while let Some(pid) = entries.next_number(self.session)? {
                if pid == 0 || pid > i32::MAX as u32 {
                    continue;
                }
                let observed = LinuxPid::open(&self.procfs.file, pid, &own_identity, self.session)
                    .and_then(|process| {
                        let found = process.matches(&self.target, pid, self.session)?;
                        Ok(found.then_some(process.start_ticks))
                    });
                self.session.check()?;
                match observed {
                    Ok(Some(start_ticks)) => {
                        if processes.len() == 256 {
                            return Err(Failure::BudgetExceeded);
                        }
                        self.session.admit(
                            0,
                            0,
                            ((processes.len() + 1) * std::mem::size_of::<ProcessStartupIdentity>())
                                as u64,
                        )?;
                        self.session.admit_result(256)?;
                        processes
                            .try_reserve_exact(1)
                            .map_err(|_| Failure::BudgetExceeded)?;
                        processes.push(ProcessStartupIdentity::Linux {
                            pid,
                            start_ticks,
                            visibility_domain_sha256: self.procfs.domain,
                        });
                    }
                    Ok(None) => {}
                    Err(Failure::Conflict) => add_code(&mut codes, Code::ProcessChanged),
                    Err(Failure::PermissionDenied) => add_code(&mut codes, Code::PermissionDenied),
                    Err(Failure::Unsupported) => add_code(&mut codes, Code::Unsupported),
                    Err(Failure::Unavailable) => add_code(&mut codes, Code::Unavailable),
                    Err(error) => return Err(error),
                }
            }
            self.verify()?;
            self.session.admit_result(512)?;
            self.session.check()?;
            ProcessEvidenceSummary::new(
                ProcessObservationMethod::LinuxProcfsV1,
                (start, now_ms()?),
                ProcessObservationCoverage::Partial,
                self.procfs.domain,
                processes,
                codes,
            )
            .map_err(|_| Failure::InternalError)
        })();
        match result {
            Ok(value) => Ok(value),
            Err(error) => self.session.fail(error),
        }
    }
    /// 参数：无；返回：编码后原祖先、scope 根及目标仍绑定原身份；调用时不得持有数据库写锁。
    pub(crate) fn verify(&self) -> Result<(), Failure> {
        match self
            .target
            .verify(self.expected, &self.procfs.boot, self.session)
        {
            Ok(()) => self.session.check(),
            Err(error) => self.session.fail(error),
        }
    }
}
fn add_code(codes: &mut Vec<Code>, code: Code) {
    if !codes.contains(&code) {
        codes.push(code);
    }
}
