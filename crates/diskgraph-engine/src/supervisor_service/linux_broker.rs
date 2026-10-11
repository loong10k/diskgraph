use super::linux_broker_launch_result::LinuxBrokerLaunchResult;
use super::{HeldInstallationImage, InstallationConfig, broker_frame, root_installation_path};
use crate::native_deadline::ClockStamp;
use crate::recovery_control::{
    ControlNotification, FrontendRead, LinuxControlRead, LinuxFrontendConnection,
    UnixForegroundOutput,
};
use crate::recovery_slot::{LinuxSupervisorTrust, SlotReservation};
use crate::{LinuxSupervisorLauncher, LinuxSupervisorProcess};
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// 持有固定安装配置、原镜像、状态域与原命名空间的Linux公开broker。来源：PF-06；无Java对等对象。
/// 分发已验证只读CLI查询，公开交付结束不代替原监督owner退休，不启用MCP或任意命令。
pub struct LinuxBroker {
    config: InstallationConfig,
    image: File,
    worker: File,
    state: File,
    user_namespace: File,
    mount_namespace: File,
}

impl LinuxBroker {
    /// 参数：path为root安装者指定配置，deadline为原启动期限；返回：完整安装准入或拒绝。
    /// 不绑定socket、不认领槽、不创建数据库；完整镜像摘要先于任何工作出生。
    pub fn open(path: &Path, deadline: Instant) -> io::Result<Self> {
        let config = InstallationConfig::open_root(path, deadline)?;
        let image = HeldInstallationImage::open(&config.cli_path, &config.cli_sha256, deadline)?
            .into_file();
        let worker =
            HeldInstallationImage::open(&config.worker_path, &config.worker_sha256, deadline)?
                .into_file();
        if worker.metadata()?.len() > 64 << 20 {
            return Err(io::Error::other("worker exceeds fixed birth image bound"));
        }
        let (state, _) = root_installation_path::open_directory(&config.state_dir, deadline)?;
        let user_namespace = File::open("/proc/self/ns/user")?;
        let mount_namespace = File::open("/proc/self/ns/mnt")?;
        root_installation_path::check(deadline)?;
        Ok(Self {
            config,
            image,
            worker,
            state,
            user_namespace,
            mount_namespace,
        })
    }

    /// 参数：once为隔离一次性验收选择，deadline为原启动期限；返回：原owner实际退休后的结束或启动拒绝。
    /// 原控制异常或恢复未完成时持续保留原owner与ACTIVE；不据客户端EOF或超时释放容量。
    pub fn serve(self, once: bool, deadline: Instant) -> io::Result<()> {
        let socket = &self.config.socket_path;
        let (_parent, _ancestors) = root_installation_path::open_directory(
            socket
                .parent()
                .ok_or_else(|| io::Error::other("fixed socket parent missing"))?,
            deadline,
        )?;
        let listener = UnixListener::bind(socket)?;
        LinuxFrontendConnection::prepare_listener(&listener).map_err(io::Error::other)?;
        listener.set_nonblocking(true)?;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o666))?;
        let identity = std::fs::symlink_metadata(socket)?;
        let mut active: Option<Box<LinuxSupervisorProcess>> = None;
        let mut unconfirmed = false;
        let mut completed_request = false;
        loop {
            // 原owner只沿私有认证流与原wait退休；控制错误锁存，但不丢弃原恢复责任。
            let mut retired = false;
            if let Some(owner) = active.as_mut() {
                if !unconfirmed {
                    let cancel = AtomicBool::new(false);
                    match owner.poll_notification(&cancel) {
                        Ok(_) => {}
                        Err(_) => unconfirmed = true,
                    }
                    if owner.read_stdout().is_err() || owner.read_stderr().is_err() {
                        unconfirmed = true;
                    }
                    if !unconfirmed {
                        match owner.poll_retirement(Instant::now() + Duration::from_millis(250)) {
                            Ok(true) => retired = true,
                            Ok(false) => {}
                            Err(_) => unconfirmed = true,
                        }
                    }
                }
                if unconfirmed {
                    // 尝试同一原child的物理处置，但异常确认不能改写全局ACTIVE或重新准入。
                    let _ = owner.poll_cleanup(Instant::now() + Duration::from_millis(250));
                    let _ = owner.read_stdout();
                    let _ = owner.read_stderr();
                }
            }
            if retired {
                active = None;
            }
            if once && completed_request && active.is_none() {
                drop(listener);
                // 只删除本次绑定的原socket；陌生替换或陈旧入口不被自动清除。
                let current = std::fs::symlink_metadata(socket)?;
                if (current.dev(), current.ino()) != (identity.dev(), identity.ino()) {
                    return Err(io::Error::other("original socket replaced"));
                }
                std::fs::remove_file(socket)?;
                return Ok(());
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    let request_deadline = Instant::now() + Duration::from_secs(30);
                    let cancel = AtomicBool::new(false);
                    let output = match UnixForegroundOutput::new(stream) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    let mut connection =
                        match LinuxFrontendConnection::new(output, self.config.frontend_uid) {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                    if active.is_some() {
                        let business = if unconfirmed {
                            diskgraph_core::BusinessError::RecoveryUnconfirmed
                        } else {
                            diskgraph_core::BusinessError::ResourceExhausted
                        };
                        let _ = failure(
                            &mut connection,
                            business,
                            "original recovery capacity unavailable",
                            request_deadline,
                            &cancel,
                        );
                        continue;
                    }
                    completed_request = true;
                    let (query, request_deadline) =
                        match receive_query(&mut connection, request_deadline, &cancel) {
                            Ok(request) => request,
                            Err(_) => {
                                let _ = send(
                                    &mut connection,
                                    3,
                                    &10i32.to_be_bytes(),
                                    request_deadline,
                                    &cancel,
                                );
                                continue;
                            }
                        };
                    let launch = self.launch(&query, request_deadline);
                    match launch {
                        Ok((mut owner, trust)) => {
                            if owner.send_materials(&trust, &cancel).is_err() {
                                unconfirmed = true;
                                let _ = send(
                                    &mut connection,
                                    3,
                                    &10i32.to_be_bytes(),
                                    request_deadline,
                                    &cancel,
                                );
                            } else {
                                let _ =
                                    deliver(&mut owner, &mut connection, request_deadline, &cancel);
                            }
                            active = Some(owner);
                        }
                        Err((error, mut owner)) => {
                            let business = super::broker_admission_error::classify(&error);
                            let _ = failure(
                                &mut connection,
                                business,
                                &error.to_string(),
                                request_deadline,
                                &cancel,
                            );
                            if owner.as_mut().is_some_and(|original| {
                                original.abort_unborn(request_deadline).is_ok()
                            }) {
                                owner = None;
                            }
                            active = owner;
                            unconfirmed = active.is_some();
                        }
                    }
                    // 同一原公开连接在此关闭；后续private恢复不能持有用户stdout/stderr。
                    drop(connection);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    if active.is_none() {
                        return Err(error);
                    }
                    // 监听失败不丢原owner；保留全局未确认记录，避免假恢复后重新准入。
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    fn launch(&self, query: &super::BrokerQuery, deadline: Instant) -> LinuxBrokerLaunchResult {
        let prepared = (|| {
            let stamp = ClockStamp::capture(deadline).map_err(io::Error::other)?;
            let mut nonce = [0; 32];
            nonce[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
            nonce[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
            let request = serde_json::to_vec(&serde_json::json!({
                "schema_version":3,"control_fd":6,"nonce":hex::encode(nonce),
                "service_uid":self.config.service_uid,"service_gid":self.config.service_gid,"frontend_uid":self.config.frontend_uid,
                "state_root":self.config.state_dir,"data_dir":self.config.data_dir,"worker_path":self.config.worker_path,
                "worker_sha256":self.config.worker_sha256,"worker_bytes":self.worker.metadata()?.len(),"deadline":stamp
            }))?;
            if request.len() > 4096 {
                return Err(io::Error::other("oversized fixed birth materials"));
            }
            let session: [u8; 32] = Sha256::digest(&request).into();
            let args = [
                self.config.cli_path.to_str().expect("validated fixed path"),
                "--json",
                "--data-dir",
                self.config.data_dir.to_str().expect("validated fixed path"),
            ]
            .into_iter()
            .map(str::to_owned)
            .chain(query.arguments()?)
            .map(|arg| CString::new(arg).map_err(io::Error::other))
            .collect::<io::Result<Vec<_>>>()?;
            let environment = vec![
                CString::new(format!(
                    "DISKGRAPH_LINUX_SUPERVISOR_BIRTH={}",
                    std::str::from_utf8(&request).map_err(io::Error::other)?
                ))
                .map_err(io::Error::other)?,
                c"DISKGRAPH_LINUX_SUPERVISOR_FD=5".to_owned(),
            ];
            let launcher = LinuxSupervisorLauncher::prepare(
                self.image.try_clone()?,
                args,
                environment,
                (self.config.service_uid, self.config.service_gid),
            )?;
            let raw = unsafe {
                libc::openat(
                    self.state.as_raw_fd(),
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
                return Err(io::Error::other("untrusted original bootstrap"));
            }
            let trust = LinuxSupervisorTrust::from_host(
                self.state.try_clone()?,
                self.user_namespace.try_clone()?,
                self.mount_namespace.try_clone()?,
            );
            let reservation = SlotReservation::acquire(file, deadline).map_err(io::Error::other)?;
            Ok((launcher, reservation, session, trust))
        })();
        let (launcher, reservation, session, trust) = prepared.map_err(|error| (error, None))?;
        launcher
            .spawn_lifecycle(reservation, session, deadline)
            .map(|owner| (owner, trust))
            .map_err(|error| error.into_parts())
    }
}

fn receive_query(
    connection: &mut LinuxFrontendConnection,
    deadline: Instant,
    cancel: &AtomicBool,
) -> io::Result<(super::BrokerQuery, Instant)> {
    let mut request = Vec::new();
    loop {
        match connection
            .poll_input_until(deadline, cancel)
            .map_err(io::Error::other)?
        {
            FrontendRead::Data(bytes) => {
                if request.len() + bytes.len() > 4096 {
                    return Err(io::Error::other("oversized broker input"));
                }
                request.extend_from_slice(bytes);
            }
            FrontendRead::Pending => std::thread::sleep(Duration::from_millis(1)),
            FrontendRead::Eof => break,
        }
    }
    if request.starts_with(b"DG_FRONT_QUERY_V1\n") {
        return super::broker_query_request::BrokerQueryRequest::decode(&request, deadline);
    }
    let bytes = request
        .strip_prefix(b"DG_FRONT_INPUT\n")
        .ok_or_else(|| io::Error::other("unsupported fixed broker command"))?;
    let original: ClockStamp = serde_json::from_slice(bytes)?;
    Ok((
        super::BrokerQuery::new("doctor", None, Vec::new(), Vec::new())?,
        original
            .adopt(Duration::from_secs(30))
            .map_err(io::Error::other)?
            .min(deadline),
    ))
}

fn send(
    connection: &mut LinuxFrontendConnection,
    kind: u8,
    bytes: &[u8],
    deadline: Instant,
    cancel: &AtomicBool,
) -> io::Result<()> {
    connection.write_output_until(&broker_frame::header(kind, bytes.len())?, deadline, cancel)?;
    connection.write_output_until(bytes, deadline, cancel)
}

fn deliver(
    owner: &mut LinuxSupervisorProcess,
    connection: &mut LinuxFrontendConnection,
    deadline: Instant,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut exit = None;
    loop {
        root_installation_path::check(deadline)?;
        match owner.poll_notification(cancel)? {
            LinuxControlRead::Frame(frame) => {
                if let ControlNotification::ForegroundEnded { exit_code, .. } = frame.notification {
                    exit = Some(exit_code);
                }
            }
            LinuxControlRead::ConfirmedEof if exit.is_none() => {
                return Err(io::Error::other("missing original foreground result"));
            }
            _ => {}
        }
        if let Some(bytes) = owner.read_stdout()? {
            send(connection, 1, bytes, deadline, cancel)?;
        }
        if let Some(bytes) = owner.read_stderr()? {
            send(connection, 2, bytes, deadline, cancel)?;
        }
        if let Some(code) = exit
            && owner.foreground_output_drained()?
        {
            send(connection, 3, &code.to_be_bytes(), deadline, cancel)?;
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn failure(
    connection: &mut LinuxFrontendConnection,
    business: diskgraph_core::BusinessError,
    message: &str,
    deadline: Instant,
    cancel: &AtomicBool,
) -> io::Result<()> {
    use diskgraph_core::{Envelope, measure_json_bounded};
    let mut envelope = Envelope::failure(business, message);
    if !matches!(measure_json_bounded(&envelope, 4095), Ok(Some(_))) {
        envelope = Envelope::failure(business, "admission diagnostic exceeds byte budget");
    }
    let line = format!("{}\n", envelope.into_json());
    send(connection, 1, line.as_bytes(), deadline, cancel)?;
    send(
        connection,
        3,
        &(business.exit_code() as i32).to_be_bytes(),
        deadline,
        cancel,
    )
}
