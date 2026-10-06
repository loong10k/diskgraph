use std::io;

/// 独立macOS扫描helper的非root派生进程限制；不更改父宿主的资源限制。
/// 来源：原生 Rust PF-06 Darwin helper入口合同；无 Java 对等对象。
pub(crate) struct MacosProcessLimit;

impl MacosProcessLimit {
    /// 参数：无；返回：原系统错误或无法可靠限制身份的Unsupported。
    /// 仅可在独立helper入口调用，必须先于协议输入和线程创建。
    pub(crate) fn install() -> io::Result<()> {
        let real = unsafe { libc::getuid() };
        let effective = unsafe { libc::geteuid() };
        if real == 0 || effective == 0 || real != effective {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "scan helper requires matching non-root identities",
            ));
        }
        let zero = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // 仅独立helper修改自身限额；失败保留紧邻系统调用的原errno。
        if unsafe { libc::setrlimit(libc::RLIMIT_NPROC, &zero) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut observed = libc::rlimit {
            rlim_cur: 1,
            rlim_max: 1,
        };
        if unsafe { libc::getrlimit(libc::RLIMIT_NPROC, &mut observed) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if observed.rlim_cur != 0 || observed.rlim_max != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "scan helper process limit could not be verified",
            ));
        }
        Ok(())
    }
}
