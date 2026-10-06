//! 父驱动的真实子进程夹具；来源：原生 Child owner，不注入退出布尔值或模拟管道。
use crate::scan_worker_driver::ScanWorkerDriver;
use diskgraph_disktree_core::scan::ScanOptions;
use diskgraph_scan_worker::{ProtocolLimits, WorkerRequest};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(super) const TARGET: &str = "driver-fixture-target";
pub(super) const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

/// 独占 driver 与隔离 fixture，失败 Drop 只作救援；来源：真实 OS 子进程生命周期。
pub(super) struct DriverFixture {
    pub(super) driver: ScanWorkerDriver,
    pub(super) directory: tempfile::TempDir,
    pub(super) deadline: Instant,
    pub(super) pid: u32,
    #[cfg(windows)]
    leader: crate::native_child::OwnedHandle,
    #[cfg(windows)]
    job: crate::native_child::OwnedHandle,
}

impl DriverFixture {
    /// 参数：mode 为固定本地夹具行为、stderr_limit 为原响应总额以内的子上界。
    /// 返回：实际启动的 child 与同一原绝对期限；缺产物直接资格失败，绝不 PATH 查找。
    pub(super) fn new(mode: &str, stderr_limit: u64) -> Self {
        Self::with_stream_limit(mode, stderr_limit, 1048576)
    }

    /// 参数：mode/stderr_limit 沿用真实夹具，stream_limit 是 stdout 与 stderr 共享原响应总额。
    /// 返回：同一 Child owner 与单次请求；不会为 stderr 另造总额度。
    pub(super) fn with_stream_limit(mode: &str, stderr_limit: u64, stream_limit: u64) -> Self {
        let program = PathBuf::from(std::env::var_os("DISKGRAPH_SCAN_DRIVER_FIXTURE").expect(
            "qualification: root must compile and provide fresh standalone driver fixture",
        ));
        assert!(program.is_absolute() && program.is_file());
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("tree");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("item"), [7; 17]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        #[cfg(target_os = "linux")]
        let child = {
            use crate::native_child::LinuxAtomicLauncher;
            use std::ffi::CString;
            use std::io::Write;
            use std::os::unix::ffi::OsStrExt;
            use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

            // Linux 使用生产原子启动器；失败或 unwind 的原 owner 不得被夹具丢弃。
            let launcher = LinuxAtomicLauncher::prepare(
                std::fs::File::open(&program).unwrap(),
                vec![
                    CString::new(program.as_os_str().as_bytes()).unwrap(),
                    CString::new(mode).unwrap(),
                    CString::new(directory.path().as_os_str().as_bytes()).unwrap(),
                ],
                Vec::new(),
            )
            .unwrap();
            let mut unwind_owner = None;
            let launched = catch_unwind(AssertUnwindSafe(|| {
                launcher.spawn(deadline, &mut || check(deadline), &mut unwind_owner)
            }));
            match launched {
                Ok(Ok(child)) => child,
                Ok(Err(failure)) => {
                    let (error, mut owner) = failure.into_parts();
                    if let Some(child) = owner.as_mut()
                        && let Err(secondary) = child.cleanup()
                    {
                        let _ = writeln!(
                            std::io::stderr(),
                            "Atomic driver fixture cleanup secondary: {secondary:?}"
                        );
                    }
                    panic!("actual Atomic driver fixture launch failed: {error:?}");
                }
                Err(payload) => {
                    // 原处置槽保留在 catch 外；清理次错不改变宿主收到的原 panic payload。
                    if let Some(child) = unwind_owner.as_mut()
                        && let Err(secondary) = child.cleanup()
                    {
                        let _ = writeln!(
                            std::io::stderr(),
                            "Atomic driver fixture unwind cleanup secondary: {secondary:?}"
                        );
                    }
                    std::mem::drop(unwind_owner);
                    resume_unwind(payload);
                }
            }
        };
        #[cfg(target_os = "macos")]
        let child = crate::native_child::UnixChild::spawn_worker(
            &program,
            &[std::ffi::OsStr::new(mode), directory.path().as_os_str()],
            &[],
            || check(deadline),
        )
        .unwrap();
        #[cfg(windows)]
        let child = {
            let mut command = std::process::Command::new(&program);
            command.arg(mode).arg(directory.path());
            crate::native_child::WindowsChild::spawn_with_input(
                &mut command,
                crate::native_child::ChildInputMode::WorkerControl,
                || check(deadline),
            )
            .unwrap()
        };
        #[cfg(windows)]
        let leader = child.duplicate_leader_for_test().unwrap();
        #[cfg(windows)]
        let job = child.duplicate_job_for_test().unwrap();
        let limits = ProtocolLimits {
            max_frame_bytes: 65536,
            max_stream_bytes: stream_limit,
            max_nodes: 4096,
            max_depth: 64,
        };
        let request = WorkerRequest::scan(&root, &options(), limits).unwrap();
        let mut child = Some(child);
        let driver =
            ScanWorkerDriver::new(&mut child, request, (TARGET, PIN), deadline, stderr_limit)
                .unwrap();
        assert!(
            child.is_none(),
            "successful configuration transfers exactly one owner"
        );
        let pid = loop {
            if let Ok(value) = std::fs::read_to_string(directory.path().join("pid"))
                && let Ok(pid) = value.parse()
            {
                break pid;
            }
            check(deadline).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        };
        Self {
            driver,
            directory,
            deadline,
            pid,
            #[cfg(windows)]
            leader,
            #[cfg(windows)]
            job,
        }
    }

    /// 参数：name 是固定阶段文件名；返回：真实子进程已写出该阶段见证。
    pub(super) fn reached(&self, name: &str) -> bool {
        self.directory.path().join(name).exists()
    }

    /// 参数：无；返回：自然释放门已写入；不执行 kill 或 cleanup。
    pub(super) fn release(&self) {
        std::fs::write(self.directory.path().join("release"), b"release").unwrap();
    }

    /// 参数：无；返回：原 leader 已真实回收；Windows 同时核对仍持有的 Job 活动数为零。
    pub(super) fn assert_reaped(&self) {
        assert!(self.pid > 0, "actual fixture PID required");
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let mut status = 0;
            assert_eq!(
                unsafe { libc::waitpid(self.pid as i32, &mut status, libc::WNOHANG) },
                -1
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
            use windows_sys::Win32::System::JobObjects::{
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
                QueryInformationJobObject,
            };
            use windows_sys::Win32::System::Threading::WaitForSingleObject;
            assert_eq!(
                unsafe { WaitForSingleObject(self.leader.as_raw(), 0) },
                WAIT_OBJECT_0
            );
            let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            assert_ne!(
                unsafe {
                    QueryInformationJobObject(
                        self.job.as_raw(),
                        JobObjectBasicAccountingInformation,
                        (&raw mut info).cast(),
                        std::mem::size_of_val(&info) as u32,
                        std::ptr::null_mut(),
                    )
                },
                0
            );
            assert_eq!(info.ActiveProcesses, 0);
        }
    }
}

impl Drop for DriverFixture {
    fn drop(&mut self) {
        // 失败路径的救援不能算自然完成或 normal permit；真正句柄由 driver 独占回收。
        let _ = std::fs::write(self.directory.path().join("release"), b"release");
    }
}

/// 参数：deadline 为 spawn 前捕获的原 Instant；返回：原期限未到，或真实 TimedOut。
pub(super) fn check(deadline: Instant) -> std::io::Result<()> {
    if Instant::now() >= deadline {
        Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
    } else {
        Ok(())
    }
}

/// 参数：无；返回：全部七项均显式设置的真实 pinned 选项，不依赖默认值遗漏字段。
pub(super) fn options() -> ScanOptions {
    ScanOptions {
        apparent_size: true,
        follow_links: true,
        include_hidden: false,
        one_filesystem: true,
        max_depth: Some(3),
        dedup_hardlinks: false,
        metric: diskgraph_disktree_core::tree::Metric::Files,
    }
}
