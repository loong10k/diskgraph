//! 逐资源 Linux 进程占用；来源：procfs held PID/fd，结果始终承认可见性边界。
use super::ProcessNativeSession;
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

impl ProcessNativeSession<'_> {
    /// 参数：实际授权根、已索引相对普通文件与扫描强 epoch；返回：逐资源启动身份与保守覆盖。
    /// 这是可信库原语，调用者必须来自 durable job 的真实 owner/scope；不解析客户端路径或授予权限。
    /// 单次/多次调用使用同会话账本，沿原认领时间；Mac/Windows 不借用 Linux 方法冒充支持。
    pub fn observe_linux_file(
        &self,
        root: &Path,
        relative: &Path,
        expected: &IndexedFileEpoch,
    ) -> Result<ProcessEvidenceSummary, Failure> {
        // 峰值包括 proc/root/target/self/枚举/PID/FD 枚举及一个被观察的 O_PATH。
        let result = self.with_handles(12, || {
            self.admit(8192, 1, 0)?;
            let start = now_ms()?;
            let procfs = LinuxProcRoot::open(&|| self.check())?;
            let target = LinuxTarget::open(root, relative, expected, &procfs.boot, self)?;
            self.check()?;
            let own = open_at(
                procfs.file.as_raw_fd(),
                c"self",
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0x08 | 0x01,
            )?;
            let own_identity = own.metadata().map_err(|_| Failure::Unavailable)?;
            self.check()?;
            let enumeration = open_at(
                procfs.file.as_raw_fd(),
                c".",
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0x08 | 0x04 | 0x02 | 0x01,
            )?;
            let mut entries = LinuxDirectory::new(enumeration, self)?;
            let mut processes = Vec::new();
            let mut codes = Vec::new();
            self.admit(0, 0, 9 * std::mem::size_of::<Code>() as u64)?;
            codes
                .try_reserve_exact(9)
                .map_err(|_| Failure::BudgetExceeded)?;
            codes.push(Code::VisibilityRestricted);
            while let Some(pid) = entries.next_number(self)? {
                if pid == 0 || pid > i32::MAX as u32 {
                    continue;
                }
                let observed =
                    LinuxPid::open(&procfs.file, pid, &own_identity, self).and_then(|process| {
                        let found = process.matches(&target, pid, self)?;
                        Ok(found.then_some(process.start_ticks))
                    });
                self.check()?;
                match observed {
                    Ok(Some(start_ticks)) => {
                        if processes.len() == 256 {
                            return Err(Failure::BudgetExceeded);
                        }
                        self.admit(
                            0,
                            0,
                            ((processes.len() + 1) * std::mem::size_of::<ProcessStartupIdentity>())
                                as u64,
                        )?;
                        self.admit_result(256)?;
                        processes
                            .try_reserve_exact(1)
                            .map_err(|_| Failure::BudgetExceeded)?;
                        processes.push(ProcessStartupIdentity::Linux {
                            pid,
                            start_ticks,
                            visibility_domain_sha256: procfs.domain,
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
            target.verify(expected, &procfs.boot, self)?;
            self.admit_result(512)?;
            self.check()?;
            ProcessEvidenceSummary::new(
                ProcessObservationMethod::LinuxProcfsV1,
                (start, now_ms()?),
                ProcessObservationCoverage::Partial,
                procfs.domain,
                processes,
                codes,
            )
            .map_err(|_| Failure::InternalError)
        });
        match result {
            Ok(value) => Ok(value),
            Err(error) => self.fail(error),
        }
    }
}
fn add_code(codes: &mut Vec<Code>, code: Code) {
    if !codes.contains(&code) {
        codes.push(code);
    }
}
