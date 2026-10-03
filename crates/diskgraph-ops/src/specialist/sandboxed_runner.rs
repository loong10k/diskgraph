//! sandboxed_runner：既有文件操作职责的原生 Rust 实现。
use crate::OpsError;
use crate::specialist::command_runner::CommandRunner;
use crate::specialist::command_spec::CommandSpec;
use crate::specialist::run_outcome::RunOutcome;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

/// 既有专家进程执行器，使用清空后指定环境和 Unix 进程组；不是 Engine 新探针执行器。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::specialist::SandboxedRunner`，保留既有语义。
/// The production runner: no shell, minimal env, bounded time and output.
pub struct SandboxedRunner;

impl SandboxedRunner {
    /// 构建既有专家工具最小环境。
    /// 参数：无。
    /// 返回：仅当前 PATH 和 LANG 的可用字符串值；不承诺全配置隔离。
    /// The environment children get when a caller has no stronger opinion:
    /// enough to find libraries and render output, nothing that leaks a
    /// caller's whole environment into a project-configured context.
    pub fn default_env() -> Vec<(String, String)> {
        default_env()
    }
}

/// 构建既有专家工具最小环境。
/// 参数：无。
/// 返回：仅当前 PATH 和 LANG 的可用字符串值；不承诺全配置隔离。
pub(super) fn default_env() -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        env.push(("PATH".into(), path.to_string_lossy().into_owned()));
    }
    if let Ok(lang) = std::env::var("LANG") {
        env.push(("LANG".into(), lang));
    }
    env
}

impl CommandRunner for SandboxedRunner {
    /// 执行原有专家命令。
    /// 参数：spec 提供固定程序、argv、清空后环境、逐流上限和重试。
    /// 返回：RunOutcome 或启动、执行错误；非 Unix 实现明确拒绝。
    fn run(&self, spec: &CommandSpec) -> Result<RunOutcome, OpsError> {
        let mut attempt = 0_u32;
        loop {
            attempt += 1;
            let outcome = run_once(spec)?;
            // A retry is for transient failures only; a timeout or a clean
            // non-zero exit is the tool's answer and is reported as such.
            let transient = !outcome.timed_out && outcome.exit_code != 0;
            if !transient || attempt > spec.retries {
                return Ok(outcome);
            }
        }
    }
}

/// 按既有平台实现执行一次专家命令。
/// 参数：spec 为完整命令设置。
/// 返回：逐流受限输出和退出状态或错误。
#[cfg(unix)]
pub(super) fn run_once(spec: &CommandSpec) -> Result<RunOutcome, OpsError> {
    use std::process::{Command, Stdio};

    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    // The child sees only what the spec names. A project file cannot conjure
    // credentials into its own cleanup tool (EC-04).
    command.env_clear();
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Put the command and ordinary descendants in their own process
        // group so timeout can close pipes inherited by background children.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn().map_err(|error| {
        OpsError::Stale(format!(
            "specialist program {} is unavailable: {error}",
            spec.program.display()
        ))
    })?;

    // Readers own their pipes so the poll loop never blocks on a quiet child.
    let cap = spec.max_output_bytes.max(1);
    let deadline = Instant::now() + Duration::from_millis(spec.timeout_ms.max(1));
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || drain(stdout_pipe, cap, deadline));
    let stderr_reader = std::thread::spawn(move || drain(stderr_pipe, cap, deadline));

    let mut timed_out = false;
    loop {
        match child.try_wait()? {
            Some(_status) => break,
            None if Instant::now() >= deadline => {
                timed_out = true;
                #[cfg(unix)]
                unsafe {
                    let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    let (stdout, stdout_truncated) = stdout_reader
        .join()
        .map_err(|_| OpsError::Stale("stdout reader failed".into()))?;
    let (stderr, stderr_truncated) = stderr_reader
        .join()
        .map_err(|_| OpsError::Stale("stderr reader failed".into()))?;
    if Instant::now() >= deadline && (stdout_truncated || stderr_truncated) {
        timed_out = true;
        // The direct child may have exited while a background descendant
        // retained stdout/stderr; its process group can still be alive.
        unsafe {
            let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    let exit_code = match child.try_wait()? {
        Some(status) => status.code().unwrap_or(-1),
        None if timed_out => -1,
        None => -1,
    };
    Ok(RunOutcome {
        exit_code,
        stdout,
        stderr,
        timed_out,
        truncated: stdout_truncated || stderr_truncated,
    })
}

/// 按既有平台实现执行一次专家命令。
/// 参数：spec 为完整命令设置。
/// 返回：逐流受限输出和退出状态或错误。
#[cfg(not(unix))]
pub(super) fn run_once(_spec: &CommandSpec) -> Result<RunOutcome, OpsError> {
    Err(OpsError::Stale(
        "unsupported: bounded specialist subprocesses are not implemented on this platform".into(),
    ))
}

/// 按现有期限读取专家管道。
/// 参数：pipe、cap、deadline 指定管道、单流上限和期限。
/// 返回：已观察的管道字节及截断状态。
/// Reads a pipe to the end, keeping at most `cap` bytes and discarding the
/// rest (so a chatty child can still finish), and reporting the discard.
#[cfg(unix)]
pub(super) fn drain(
    pipe: Option<impl Read + std::os::fd::AsRawFd>,
    cap: usize,
    deadline: Instant,
) -> (Vec<u8>, bool) {
    let mut buffer = Vec::new();
    let Some(mut pipe) = pipe else {
        return (buffer, false);
    };
    let fd = std::os::fd::AsRawFd::as_raw_fd(&pipe);
    // SAFETY: fd stays owned by this reader for its whole lifetime.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return (buffer, true);
        }
    }
    let mut chunk = [0_u8; 8_192];
    loop {
        if Instant::now() >= deadline {
            return (buffer, true);
        }
        match pipe.read(&mut chunk) {
            Ok(0) => return (buffer, false),
            Ok(read) => {
                if buffer.len() < cap {
                    let keep = read.min(cap - buffer.len());
                    buffer.extend_from_slice(&chunk[..keep]);
                    if keep < read {
                        // The cap is hit; keep draining so the child is not
                        // blocked on a full pipe, but the tail is lost.
                        drain_rest(&mut pipe, deadline);
                        return (buffer, true);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return (buffer, true),
        }
    }
}

/// 按原有策略排出剩余管道。
/// 参数：pipe 为原有管道；deadline 为剩余排空截止时刻。
/// 返回：无返回值；按既有期限/错误边界停止排空。
#[cfg(unix)]
pub(super) fn drain_rest(pipe: &mut impl Read, deadline: Instant) {
    let mut chunk = [0_u8; 8_192];
    while Instant::now() < deadline {
        match pipe.read(&mut chunk) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return,
        }
    }
}
