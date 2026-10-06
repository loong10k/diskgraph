use std::io;
#[cfg(target_os = "macos")]
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, ExitStatus};

/// 独占标准或原生出生的未回收 leader，完成后缓存原退出状态以避免旧 PID 再访问。
/// 来源：原生 Rust PF-06 Unix retained leader 合同；无 Java 对等对象。
pub(super) enum UnixLeader {
    Standard(Child),
    #[cfg(target_os = "macos")]
    Native {
        pid: libc::pid_t,
        status: Option<ExitStatus>,
    },
}

impl UnixLeader {
    /// 预留原生PID空槽。参数：无；返回：只有出生成功后才有wait资格的预备状态。
    #[cfg(target_os = "macos")]
    pub(super) fn prepare_native() -> Self {
        Self::Native {
            pid: 0,
            status: None,
        }
    }

    /// 接管唯一标准 Child。参数：child 为本次实际子进程；返回：不新增句柄的 owner。
    pub(super) fn from_standard(child: Child) -> Self {
        Self::Standard(child)
    }

    /// 接管成功原生出生的正 PID。参数：pid 为调用方唯一拥有的原子出生结果。
    /// 返回：未回收 leader，不修改信号策略；须立即放入完整管道与恢复 owner。
    /// 安全性：调用方必须保留此 PID 的独占 wait 责任，禁止 auto-reap 或其他线程抢先 wait。
    #[cfg(target_os = "macos")]
    pub(super) unsafe fn from_native(pid: libc::pid_t) -> Self {
        Self::Native { pid, status: None }
    }

    /// 借用原未回收身份。参数：无；返回：本次 leader 的 PID，完成后仅供缓存诊断。
    pub(super) fn id(&self) -> u32 {
        match self {
            Self::Standard(child) => child.id(),
            #[cfg(target_os = "macos")]
            Self::Native { pid, .. } => *pid as u32,
        }
    }

    /// 消费原 leader 的实际 wait，重复调用仅返回已缓存的退出状态。
    /// 参数：无；返回：实际退出状态或保留原 errno 的 I/O 错误；只等待本 owner，不 wait(-1)。
    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        match self {
            Self::Standard(child) => child.wait(),
            #[cfg(target_os = "macos")]
            Self::Native { pid, status } => {
                // 0/-1/负组号会扩大 waitpid 的目标，未出生或损坏槽不能消费其他 child。
                if *pid <= 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "native leader has no owned positive pid",
                    ));
                }
                if let Some(completed) = status {
                    return Ok(*completed);
                }
                loop {
                    let mut native_status = 0;
                    // 安全性：原 leader 未被回收且独占；waitpid 只消费该原正 PID。
                    let waited = unsafe { libc::waitpid(*pid, &mut native_status, 0) };
                    if waited == *pid {
                        let completed = ExitStatus::from_raw(native_status);
                        *status = Some(completed);
                        return Ok(completed);
                    }
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted {
                        return Err(error);
                    }
                }
            }
        }
    }
}
