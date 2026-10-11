//! 原生纵向测试的独立降权前端；失败处置不作为生产退休证据。
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// 保持原前端Child和原stdout直到实际EOF与wait；不按PID字符串重新认领。
pub(super) struct OriginalFrontend {
    child: Child,
    stdout_eof: bool,
}
impl OriginalFrontend {
    /// 参数：固定已复制的root夹具镜像及原监听端；返回：真实专用UID前端的唯一owner。
    pub(super) fn spawn(image: &Path, socket: &Path) -> Self {
        let mut command = Command::new(image);
        command.args(["frontend", socket.to_str().unwrap(), "doctor"]);
        unsafe {
            command.pre_exec(|| {
                if libc::setgroups(0, std::ptr::null()) != 0
                    || libc::setgid(23112) != 0
                    || libc::setuid(23112) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let owner = Self {
            child,
            stdout_eof: false,
        };
        // 先构造原处置owner，再准备非阻塞输出；失败也不遗留无人wait的前端。
        let fd = owner.child.stdout.as_ref().unwrap().as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        owner
    }

    /// 参数：实际stdout收集器；返回：真实公共EOF已到达且原Child实际wait成功才为true。
    /// 每轮只读4096字节，累计128KiB；调用者负责原绝对期限。
    pub(super) fn poll_capture(&mut self, response: &mut Vec<u8>) -> bool {
        if !self.stdout_eof {
            let mut bytes = [0; 4096];
            match self.child.stdout.as_mut().unwrap().read(&mut bytes) {
                Ok(0) => self.stdout_eof = true,
                Ok(count) => response.extend_from_slice(&bytes[..count]),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => panic!("original frontend stdout: {error}"),
            }
            assert!(response.len() <= 128 << 10);
        }
        let status = self.child.try_wait().unwrap();
        if let Some(status) = status {
            assert!(status.success(), "original frontend failed: {status:?}");
        }
        self.stdout_eof && status.is_some()
    }
}
impl Drop for OriginalFrontend {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        // 仅失败夹具处置同一原Child；kill/wait不得写作正常公共EOF或监督恢复证明。
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
