use super::linux_guardian_shutdown::LinuxGuardianShutdown;
use super::{
    HeldInstallationImage, InstallationConfig, LinuxExecutorRequest, linux_broker,
    root_installation_path,
};
use crate::native_child::LinuxGuardianIdentity;
use crate::recovery_control::{LinuxFrontendConnection, UnixForegroundOutput};
use crate::recovery_slot::SlotReservation;
use crate::{LinuxSupervisorLauncher, LinuxSupervisorProcess, ScanWorkerHostConfig};
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// 单连接guardian的可信root启动层与专用服务身份退休循环；来源：PF-06，无Java对等对象。
/// 不建立Engine，不读取客户文件；用户executor始终保留原Engine和pending资源。
pub struct LinuxGuardian;

impl LinuxGuardian {
    /// 参数：config_path为固定root配置，stream为OS已接受的原Unix连接，deadline为原启动期限。
    /// 返回：认证请求完成且原executor实际退休后的结果；异常控制保留原owner与ACTIVE。
    /// 此入口用于独立单连接服务，不在已运行的多线程broker里改变进程身份。
    pub fn serve_accepted(
        config_path: &Path,
        stream: UnixStream,
        deadline: Instant,
    ) -> io::Result<()> {
        let stopped = LinuxGuardianShutdown::install()?;
        validate_original_stream(&stream)?;
        let config = InstallationConfig::open_root(config_path, deadline)?;
        let output = UnixForegroundOutput::new(stream)?;
        let mut connection =
            LinuxFrontendConnection::new(output, config.frontend_uid).map_err(io::Error::other)?;
        let cancel = AtomicBool::new(false);
        let (query, deadline) = linux_broker::receive_query(&mut connection, deadline, stopped)?;
        let (image_path, image_digest) = if query.is_mcp() {
            (
                config
                    .mcp_path
                    .as_ref()
                    .ok_or_else(|| io::Error::other("fixed MCP image not installed"))?,
                config
                    .mcp_sha256
                    .as_ref()
                    .ok_or_else(|| io::Error::other("fixed MCP digest not installed"))?,
            )
        } else {
            (&config.cli_path, &config.cli_sha256)
        };
        let image = HeldInstallationImage::open(image_path, image_digest, deadline)?.into_file();
        let worker =
            HeldInstallationImage::open(&config.worker_path, &config.worker_sha256, deadline)?
                .into_file();
        let expected = ScanWorkerHostConfig::from_expected_image(
            hex::decode(&config.worker_sha256)
                .map_err(io::Error::other)?
                .try_into()
                .map_err(|_| io::Error::other("invalid installed worker digest"))?,
            worker.metadata()?.len(),
        )
        .map_err(io::Error::other)?;
        let identity = connection.unix_identity();
        let request = LinuxExecutorRequest::encode_from_host(
            (identity.uid(), identity.gid()),
            identity.supplementary_groups(),
            &config.data_dir,
            &expected,
            deadline,
        )
        .map_err(io::Error::other)?;
        let session: [u8; 32] = Sha256::digest(&request).into();
        let arguments = if query.is_mcp() {
            vec![
                image_path.as_os_str().to_owned(),
                "--data-dir".into(),
                config.data_dir.as_os_str().to_owned(),
                "--transport".into(),
                "stdio".into(),
                "--profile".into(),
                "read-full".into(),
            ]
        } else {
            [
                config.cli_path.to_str().expect("validated fixed path"),
                "--json",
                "--data-dir",
                config.data_dir.to_str().expect("validated fixed path"),
            ]
            .into_iter()
            .map(std::ffi::OsString::from)
            .chain(query.native_arguments()?)
            .collect()
        };
        let arguments = arguments
            .into_iter()
            .map(|arg| CString::new(arg.as_os_str().as_bytes()).map_err(io::Error::other))
            .collect::<io::Result<Vec<_>>>()?;
        let environment = vec![
            CString::new(format!(
                "DISKGRAPH_LINUX_EXECUTOR_BIRTH={}",
                std::str::from_utf8(&request).map_err(io::Error::other)?
            ))
            .map_err(io::Error::other)?,
            c"DISKGRAPH_LINUX_EXECUTOR_FD=5".to_owned(),
        ];
        let launcher = LinuxSupervisorLauncher::prepare_user_executor(
            image,
            arguments,
            environment,
            identity,
        )?
        .with_worker_image(worker, &expected, deadline)?;
        let reservation = original_reservation(&config, deadline)?;
        let mut owner = match launcher.spawn_executor_lifecycle(reservation, session, deadline) {
            Ok(owner) => owner,
            Err(failure) => {
                let (primary, original) = failure.into_parts();
                if let Some(mut owner) = original
                    && owner.abort_unborn(deadline).is_err()
                {
                    // 出生后失败仍由同一进程降权并保管原责任，不进入公开业务分发。
                    let _ = LinuxGuardianIdentity::enter(config.service_uid, config.service_gid);
                    drop(connection);
                    retain_original(owner, true);
                }
                return Err(primary);
            }
        };
        let authenticated = send_authentication(&mut owner, deadline, &cancel);
        // 原child已出生；root握手失败只关闭公开交付，不撤销原私有认证或退休责任。
        let greeting = connection.write_output_until(
            super::linux_guardian_frontend::ROOT_HELLO,
            deadline,
            stopped,
        );
        let sealed = LinuxGuardianIdentity::enter(config.service_uid, config.service_gid);
        if authenticated.is_err() || sealed.is_err() {
            // 认证/身份切换失败不交付公开业务结果，仍保留原child与全局槽。
            drop(connection);
            retain_original(owner, true);
            return Err(io::Error::other("guardian birth or identity unconfirmed"));
        }
        // 从此仅专用服务身份处置控制/管道/原pidfd；没有客户文件、Engine或root目录句柄。
        let result = greeting.and_then(|()| {
            if query.is_mcp() {
                super::linux_mcp_session::deliver(&mut owner, &mut connection, stopped)
            } else {
                linux_broker::deliver(&mut owner, &mut connection, deadline, stopped)
            }
        });
        // 同一公开输入结束必须关闭同一MCP stdin；仍等待原私有完成与实际child wait。
        let _ = owner.finish_stdin();
        drop(connection);
        // 公开连接的超时/断开不污染私有恢复资格；真实私有错误由原reader锁存后独立确认。
        retain_original(owner, false);
        result
    }
}

fn validate_original_stream(stream: &UnixStream) -> io::Result<()> {
    let mut kind: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&kind) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut kind as *mut libc::c_int).cast(),
            &mut length,
        )
    } != 0
        || kind != libc::SOCK_STREAM
        || length as usize != std::mem::size_of_val(&kind)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "original Unix stream required",
        ));
    }
    stream.local_addr()?;
    stream.peer_addr()?;
    Ok(())
}

fn original_reservation(
    config: &InstallationConfig,
    deadline: Instant,
) -> io::Result<SlotReservation> {
    let (directory, _ancestors) =
        root_installation_path::open_directory(&config.state_dir, deadline)?;
    let raw = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            c"bootstrap.slot".as_ptr(),
            libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(raw) };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "untrusted original bootstrap",
        ));
    }
    SlotReservation::acquire(file, deadline).map_err(io::Error::other)
}

fn send_authentication(
    owner: &mut LinuxSupervisorProcess,
    deadline: Instant,
    cancel: &AtomicBool,
) -> io::Result<()> {
    loop {
        root_installation_path::check(deadline)?;
        if owner.poll_executor_authentication(cancel)? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

// 保留原进程和槽；EOF、清理信号或异常wait均不能自行转换为CLEAN。
#[allow(clippy::boxed_local)] // 保留出生前预分配的原owner，不为退休路径重新分配或复制进程责任。
fn retain_original(mut owner: Box<LinuxSupervisorProcess>, mut unconfirmed: bool) {
    let cancel = AtomicBool::new(false);
    loop {
        let until = Instant::now() + Duration::from_millis(250);
        if owner.abort_unborn(until).is_ok() {
            return;
        }
        if !unconfirmed && owner.poll_notification(&cancel).is_err() {
            unconfirmed = true;
        }
        if owner.read_stdout().is_err() || owner.read_stderr().is_err() {
            unconfirmed = true;
        }
        if !unconfirmed {
            match owner.poll_retirement(until) {
                Ok(true) => return,
                Ok(false) => {}
                Err(_) => unconfirmed = true,
            }
        }
        if unconfirmed {
            let _ = owner.poll_cleanup(until);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
