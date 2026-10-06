use super::ChildError;
use super::ChildInputMode;
use super::unix_child_setup::UnixChildSetup;
use super::unix_control_channel::UnixControlChannel;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

/// 出生前独占标准通道及父端控制socket，所有子端FD至少为3且禁止额外exec继承。
/// 来源：原生 Rust PF-06 macOS native spawn 准备合同；无 Java 对等对象。
pub(super) struct MacosNativePipes {
    pub(super) control: UnixControlChannel,
    pub(super) input: File,
    pub(super) stdout: File,
    pub(super) output: File,
    pub(super) stderr: File,
    pub(super) diagnostic: File,
}

impl MacosNativePipes {
    /// 建立全部通道。参数：input必须为WorkerControl，检查点沿原请求取消和期限；返回：出生前可失败的独占资源，不修改宿主信号策略。
    pub(super) fn prepare(
        input: ChildInputMode,
        checkpoint: &mut impl FnMut() -> Result<(), crate::EngineError>,
    ) -> Result<Self, crate::EngineError> {
        let _gate = super::native_birth_gate::NativeBirthGate::acquire(checkpoint)?;
        Self::prepare_channels(input)
            .map_err(crate::scan_worker_error_projection::ScanWorkerErrorProjection::child)
    }

    fn prepare_channels(input: ChildInputMode) -> Result<Self, ChildError> {
        // worker协议必须有真实控制输入；Null仅用于旧探针，不可启动扫描协议。
        if input == ChildInputMode::Null {
            return Err(ChildError::Unsupported(
                "native scanner requires worker control input",
            ));
        }
        let (parent, child) = UnixStream::pair()
            .map_err(|error| ChildError::io("prepare native control socket", error))?;
        child
            .set_nonblocking(false)
            .map_err(|error| ChildError::io("prepare blocking native input", error))?;
        let control = UnixControlChannel::from_stream(parent)?;
        let input = above_stdio(File::from(OwnedFd::from(child)))?;
        let (stdout, output) = pipe()?;
        let (stderr, diagnostic) = pipe()?;
        UnixChildSetup::initialize(
            [Some(stdout.as_raw_fd()), Some(stderr.as_raw_fd())],
            false,
            0,
        )?;
        Ok(Self {
            control,
            input,
            stdout,
            output,
            stderr,
            diagnostic,
        })
    }
}

fn pipe() -> Result<(File, File), ChildError> {
    let mut channels = [-1; 2];
    // 安全性：固定双元素输出；成功后立即接管两原FD，不在中间执行可失败逻辑。
    if unsafe { libc::pipe(channels.as_mut_ptr()) } < 0 {
        return Err(ChildError::io(
            "prepare native output pipe",
            io::Error::last_os_error(),
        ));
    }
    let read = unsafe { File::from_raw_fd(channels[0]) };
    let write = unsafe { File::from_raw_fd(channels[1]) };
    #[cfg(test)]
    observe_raw_pipe(channels);
    Ok((above_stdio(read)?, above_stdio(write)?))
}

fn above_stdio(original: File) -> Result<File, ChildError> {
    // 安全性：原File在复制期间保活；新FD至少3并原子带CLOEXEC，映射不会与0/1/2冲突。
    let fd = unsafe { libc::fcntl(original.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err(ChildError::io(
            "prepare owned native channel",
            io::Error::last_os_error(),
        ));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(test)]
#[path = "macos_pipe_test_hook.rs"]
mod macos_pipe_test_hook;
#[cfg(test)]
use macos_pipe_test_hook::observe_raw_pipe;
#[cfg(test)]
pub(super) use macos_pipe_test_hook::set_raw_pipe_hook;
