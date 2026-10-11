use std::io;
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 唯一公共前台输出端；来源：PF-06 前台EOF与原恢复责任分离合同。
/// 不认证调用者或建立服务信任；受信宿主交付未克隆的原输出端，恢复owner始终独立持有。
pub struct UnixForegroundOutput {
    stream: Option<UnixStream>,
    failed: bool,
}

impl UnixForegroundOutput {
    /// 参数：stream为宿主已准入且不共享的原公共输出端；返回：有限非阻塞输出owner或原错误。
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        #[cfg(target_os = "macos")]
        {
            let enabled: libc::c_int = 1;
            let result = unsafe {
                libc::setsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_NOSIGPIPE,
                    (&enabled as *const libc::c_int).cast(),
                    std::mem::size_of_val(&enabled) as _,
                )
            };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Self {
            stream: Some(stream),
            failed: false,
        })
    }

    /// 参数：bytes为至多4096字节原输出块，deadline/cancel为本次原交付预算；返回：完整写入或锁存失败。
    /// 部分写入后失败禁止重发；不分配完整响应，不延长deadline，不持有或释放恢复资源。
    pub fn write_chunk_until(
        &mut self,
        bytes: &[u8],
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        if self.failed || self.stream.is_none() || bytes.is_empty() || bytes.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "foreground output unavailable or invalid chunk",
            ));
        }
        let result = self.write_inner(bytes, deadline, cancel);
        if result.is_err() {
            self.failed = true;
            // 原交付错误优先保留；同步关闭公共端，不能让取消后的客户端继续等待原资源退休。
            let _ = self.finish();
        }
        result
    }

    fn write_inner(&self, bytes: &[u8], deadline: Instant, cancel: &AtomicBool) -> io::Result<()> {
        let stream = self.stream.as_ref().unwrap();
        let mut offset = 0;
        while offset < bytes.len() {
            if cancel.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "foreground delivery cancelled",
                ));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "original foreground delivery deadline",
                ));
            }
            #[cfg(target_os = "linux")]
            let flags = libc::MSG_NOSIGNAL;
            #[cfg(target_os = "macos")]
            let flags = 0;
            let written = unsafe {
                libc::send(
                    stream.as_raw_fd(),
                    bytes[offset..].as_ptr().cast(),
                    bytes.len() - offset,
                    flags,
                )
            };
            if written > 0 {
                offset += written as usize;
                continue;
            }
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "foreground output made no progress",
                ));
            }
            let error = io::Error::last_os_error();
            if !matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return Err(error);
            }
            std::thread::sleep(
                Duration::from_millis(1).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
        Ok(())
    }

    /// 参数：无；返回：关闭原公共端的结果；实际EOF不表示监督进程或原恢复资源已经退休。
    pub fn finish(&mut self) -> io::Result<()> {
        match self.stream.take() {
            Some(stream) => stream.shutdown(Shutdown::Both),
            None => Ok(()),
        }
    }
}
