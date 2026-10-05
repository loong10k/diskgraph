use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

/// 独立 native fixture 的测试 owner；来源：std::process 与 Linux 真实退出观察。
/// Drop 的异常 SIGKILL 仅用于测试收场，绝不计作正常退出证据。
pub(crate) struct ProbeChild {
    child: Child,
    directory: tempfile::TempDir,
    reaped: bool,
}

impl ProbeChild {
    /// 启动 root 显式编译的 fixture；参数：mode 为固定模式；返回：本次唯一子进程 owner。
    pub(crate) fn spawn(mode: &str) -> Self {
        let binary = std::env::var_os("DG_LINUX_GROUP_PROBE_BINARY")
            .expect("root must compile and provide DG_LINUX_GROUP_PROBE_BINARY explicitly");
        let binary = Path::new(&binary);
        assert!(
            binary.is_absolute() && binary.is_file(),
            "qualified native fixture artifact"
        );
        let directory = tempfile::tempdir().unwrap();
        let mut command = Command::new(binary);
        command
            .args([mode])
            .arg(directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // leader_exit 的真正 main 自己 setsid；先 setpgid 会使它无权创建 session。
        // 其它模式的 private namespace/fork 仅在此自有组内，异常收场不影响宿主。
        if mode != "leader_exit" {
            command.process_group(0);
        }
        let child = command.spawn().unwrap();
        Self {
            child,
            directory,
            reaped: false,
        }
    }

    /// 查询真实子进程 PID；参数：无；返回：std 持有的原 leader PID。
    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    /// 等待真实标记；参数：name 为固定标记；返回：存在时完成，不创建伪 marker。
    pub(crate) fn wait_file(&self, name: &str) {
        let started = Instant::now();
        while !self.directory.path().join(name).exists() {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "fixture marker {name}"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// 读取 fixture 标量；参数：name 为固定文件；返回：真正 main/pthread 写入的数字。
    pub(crate) fn scalar(&self, name: &str) -> u64 {
        self.wait_file(name);
        std::fs::read_to_string(self.directory.path().join(name))
            .unwrap()
            .parse()
            .unwrap()
    }

    /// 验证单个原线程 heartbeat 实际推进；参数：index 指定线程；返回：新序号。
    pub(crate) fn advancing(&self, index: usize) -> u64 {
        let name = format!("heartbeat_{index}");
        let prior = self.scalar(&name);
        let started = Instant::now();
        loop {
            let next = self.scalar(&name);
            if next > prior {
                return next;
            }
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "live thread heartbeat"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// 自然释放全部原 pthread；参数：无；返回：释放标记已提交，未发送任何信号。
    pub(crate) fn release(&self) {
        std::fs::write(self.directory.path().join("release"), b"release").unwrap();
    }

    /// 等待实际 OS 退出；参数：无；返回：原退出码，不调用 kill/cleanup 冒充成功。
    pub(crate) fn wait(&mut self) -> ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                self.reaped = true;
                return status;
            }
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "native fixture exit watchdog"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// 读取自然退出后的有界原输出；参数：无；返回：真实 JSON/诊断与退出码。
    pub(crate) fn output(&mut self) -> Output {
        let status = self.wait();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        self.child
            .stdout
            .take()
            .unwrap()
            .take(65537)
            .read_to_end(&mut stdout)
            .unwrap();
        self.child
            .stderr
            .take()
            .unwrap()
            .take(65537)
            .read_to_end(&mut stderr)
            .unwrap();
        assert!(
            stdout.len() <= 65536 && stderr.len() <= 65536,
            "bounded probe output"
        );
        Output {
            status,
            stdout,
            stderr,
        }
    }
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        if !self.reaped {
            // 安全性：leader 仍是本 owner 未回收 child；只异常终止这个隔离测试组。
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// 验证 pidfd 的真实可读状态；参数：fd 为已持有描述符；返回：退出通知或实际系统错误。
pub(crate) fn pidfd_ready(fd: i32) -> io::Result<bool> {
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut poll, 1, 0) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if poll.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
        return Err(io::Error::other("pidfd poll failed"));
    }
    Ok(poll.revents & (libc::POLLIN | libc::POLLHUP) != 0)
}
