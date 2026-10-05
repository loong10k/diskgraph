//! macOS owned-failure 的真实子进程材料；来源：既有 standalone driver fixture 与 retained wait。
#![cfg(target_os = "macos")]

use crate::native_child::UnixChild;
use diskgraph_scan_worker::{ProtocolLimits, WorkerRequest};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 独占隔离目录和尚未转移的真实 Child，外部 reap 只消费本案已观察退出的 leader。
/// 来源：原生 Rust PF-06 失败 owner 移交测试，不模拟清理返回值或建立数值信号后备。
pub(super) struct OwnedFailureFixture {
    child: Option<UnixChild>,
    directory: tempfile::TempDir,
    pub(super) deadline: Instant,
    pub(super) pid: i32,
}

impl OwnedFailureFixture {
    /// 参数：无；返回：真实已启动且尚阻塞于 Request 的独立子进程，缺产物直接资格失败。
    pub(super) fn new() -> Self {
        let program = PathBuf::from(
            std::env::var_os("DISKGRAPH_SCAN_DRIVER_FIXTURE")
                .expect("qualification: fresh standalone driver fixture is required"),
        );
        assert!(program.is_absolute() && program.is_file());
        let directory = tempfile::tempdir().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let child = UnixChild::spawn_worker(
            &program,
            &[
                std::ffi::OsStr::new("checkpoint"),
                directory.path().as_os_str(),
            ],
            &[],
            || check(deadline),
        )
        .unwrap();
        let mut fixture = Self {
            child: Some(child),
            directory,
            deadline,
            pid: 0,
        };
        fixture.pid = loop {
            if let Ok(text) = std::fs::read_to_string(fixture.directory.path().join("pid")) {
                if let Ok(pid) = text.parse::<i32>() {
                    assert!(pid > 0);
                    break pid;
                }
            }
            check(deadline).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(
            !fixture.child.as_mut().unwrap().poll().unwrap(),
            "real child is alive before handoff"
        );
        fixture
    }

    /// 参数：无；返回：仍沿同一目录和全部默认选项的合法 Request，不创建新期限。
    pub(super) fn request(&self) -> WorkerRequest {
        WorkerRequest::scan(
            self.directory.path(),
            &Default::default(),
            ProtocolLimits {
                max_frame_bytes: 65536,
                max_stream_bytes: 1048576,
                max_nodes: 4096,
                max_depth: 64,
            },
        )
        .unwrap()
    }

    /// 参数：无；返回：唯一实际 Child，测试夹具不保留第二份句柄或虚拟 owner。
    pub(super) fn take_child(&mut self) -> UnixChild {
        self.child
            .take()
            .expect("actual Child can be transferred only once")
    }

    /// 参数：无；返回：真实控制 EOF 导致退出91，随后实际 waitpid 消费原 leader 的资格。
    /// 本夹具从未发送 Request，无成功 End；不把此资格丢失说成仍活动进程的杀死失败。
    pub(super) fn external_reap(&mut self) {
        let child = self.child.as_mut().unwrap();
        child.request_control_close().unwrap();
        loop {
            check(self.deadline).unwrap();
            let _ = child.read_stdout().unwrap();
            let _ = child.read_stderr().unwrap();
            if child.poll().unwrap() && child.stdout_eof() && child.stderr_eof() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            child.exit_code(),
            Some(91),
            "real fixture reports Request EOF, not a successful result"
        );
        let mut status = 0;
        let waited = loop {
            // 只消费已由原 owner 的 WNOWAIT 确认退出的本案 child；不等待或信号任意其它进程。
            let result = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                check(self.deadline).unwrap();
                continue;
            }
            break result;
        };
        assert_eq!(waited, self.pid);
        assert!(libc::WIFEXITED(status));
        assert_eq!(libc::WEXITSTATUS(status), 91);
    }

    /// 参数：无；返回：本夹具仍拥有的独立 child 当前确实活动，不允许别案失败误杀。
    pub(super) fn assert_live(&mut self) {
        assert!(!self.child.as_mut().unwrap().poll().unwrap());
    }

    /// 参数：无；返回：对仍拥有的 child 显式执行真实清理，成功并不来自 Drop 救援。
    pub(super) fn cleanup(&mut self) {
        self.child.as_mut().unwrap().cleanup().unwrap();
    }

    /// 参数：无；返回：cleanup 成功后的真实 ECHILD 见证，不把 wrapper None 自身当退出证明。
    pub(super) fn assert_reaped(&self) {
        let mut status = 0;
        let result = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
        assert_eq!(result, -1);
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}

impl Drop for OwnedFailureFixture {
    fn drop(&mut self) {
        // 错误测试也释放自己目录中的真实 hold 门；唯一 Child 若未转移仍由原 RAII 处置。
        let _ = std::fs::write(self.directory.path().join("release"), b"release");
    }
}

/// 参数：deadline 为 spawn 前的原请求起点推导值；返回：原期限资格或 TimedOut，不刷新时间。
pub(super) fn check(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        Err(io::ErrorKind::TimedOut.into())
    } else {
        Ok(())
    }
}
