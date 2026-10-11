//! 当前CLI/worker的原子监督纵向资格；不以固定C角色代替业务。
use super::linux_supervisor_launcher::LinuxSupervisorLauncher;
use crate::native_deadline::ClockStamp;
use crate::recovery_control::{ControlNotification, LinuxControlRead, UnixForegroundOutput};
use crate::recovery_slot::{LinuxSupervisorTrust, SlotReservation};
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires current CLI/worker images and isolated root installation; mandatory native CI"]
fn original_launcher_runs_actual_doctor_and_retires_both_original_slots() {
    assert_eq!(unsafe { libc::geteuid() }, 0);
    // /var/lib祖先由实际root保护；不把runner用户目录或/tmp声明为受信安装树。
    let root = tempfile::Builder::new()
        .prefix("diskgraph-doctor-qualified-")
        .tempdir_in("/var/lib")
        .unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let cli = root.path().join("diskgraph");
    let worker = root.path().join("diskgraph-scan-worker");
    copy_current_image("DG_LINUX_SUPERVISOR_CLI", &cli);
    copy_current_image("DG_LINUX_SUPERVISOR_WORKER", &worker);
    let state = root.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();
    let bootstrap = state.join("bootstrap.slot");
    let service_slot = state.join("uid_23111.slot");
    for path in [&bootstrap, &service_slot] {
        std::fs::write(path, b"DGSL01C\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    chown_service(&service_slot);
    let data = root.path().join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();
    chown_service(&data);
    let deadline = Instant::now() + Duration::from_secs(30);
    let stamp = ClockStamp::capture(deadline).unwrap();
    let mut nonce = [0; 32];
    File::open("/dev/urandom")
        .unwrap()
        .read_exact(&mut nonce)
        .unwrap();
    let worker_bytes = std::fs::metadata(&worker).unwrap().len();
    assert!(worker_bytes <= 64 << 20);
    let digest = file_digest(&worker);
    let request = serde_json::to_vec(&serde_json::json!({
        "schema_version": 3, "control_fd": 6, "nonce": hex::encode(nonce),
        "service_uid": 23111, "service_gid": 23111, "frontend_uid": 23112,
        "state_root": state, "data_dir": data, "worker_path": worker,
        "worker_sha256": hex::encode(digest), "worker_bytes": worker_bytes,
        "deadline": stamp
    }))
    .unwrap();
    assert!(request.len() <= 4096);
    let session: [u8; 32] = Sha256::digest(&request).into();
    let trust = LinuxSupervisorTrust::from_host(
        File::open(&state).unwrap(),
        File::open("/proc/self/ns/user").unwrap(),
        File::open("/proc/self/ns/mnt").unwrap(),
    );
    let reservation = SlotReservation::acquire(
        File::options()
            .read(true)
            .write(true)
            .open(&bootstrap)
            .unwrap(),
        deadline,
    )
    .unwrap();
    let launcher = LinuxSupervisorLauncher::prepare(
        File::open(&cli).unwrap(),
        vec![
            CString::new(cli.to_str().unwrap()).unwrap(),
            c"--json".to_owned(),
            c"--data-dir".to_owned(),
            CString::new(data.to_str().unwrap()).unwrap(),
            c"doctor".to_owned(),
        ],
        vec![
            CString::new(format!(
                "DISKGRAPH_LINUX_SUPERVISOR_BIRTH={}",
                std::str::from_utf8(&request).unwrap()
            ))
            .unwrap(),
            c"DISKGRAPH_LINUX_SUPERVISOR_FD=5".to_owned(),
        ],
        (23111, 23111),
    )
    .unwrap();
    let mut owner = launcher
        .spawn_lifecycle(reservation, session, deadline)
        .unwrap();
    let cancel = AtomicBool::new(false);
    owner.send_materials(&trust, &cancel).unwrap();
    let (public, mut reader) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    owner
        .attach_foreground_output(UnixForegroundOutput::new(public).unwrap())
        .unwrap();
    let mut response = Vec::new();
    let mut diagnostics = Vec::new();
    let mut notifications = Vec::new();
    let mut private_eof = false;
    let mut public_eof = false;
    while !public_eof {
        assert!(
            Instant::now() < deadline,
            "doctor delivery expired: {diagnostics:?}"
        );
        if !private_eof {
            match owner.poll_notification(&cancel).unwrap() {
                LinuxControlRead::Frame(frame) => notifications.push(frame.notification),
                LinuxControlRead::ConfirmedEof => private_eof = true,
                LinuxControlRead::Pending => {}
            }
        }
        owner.forward_stdout_until(deadline, &cancel).unwrap();
        if let Some(bytes) = owner.read_stderr().unwrap() {
            diagnostics.extend_from_slice(bytes);
        }
        let mut bytes = [0; 4096];
        match reader.read(&mut bytes) {
            Ok(0) => public_eof = true,
            Ok(count) => response.extend_from_slice(&bytes[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("original public read: {error}"),
        }
        assert!(response.len() + diagnostics.len() <= 128 << 10);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(notifications.contains(&ControlNotification::Ready {}));
    assert!(
        notifications.contains(&ControlNotification::ForegroundEnded {
            exit_code: 0,
            panicked: false
        })
    );
    let document: serde_json::Value = serde_json::from_slice(&response).unwrap();
    assert_eq!(document["ok"], true, "{document}; stderr={diagnostics:?}");
    assert!(
        data.join("diskgraph-control.sqlite").exists(),
        "actual doctor must open its real isolated databases"
    );
    // 公共EOF不能代替原wait；根父侧尚未轮询退休，bootstrap必须仍为ACTIVE。
    assert_eq!(std::fs::read(&bootstrap).unwrap(), b"DGSL01A\n");
    loop {
        assert!(
            Instant::now() < deadline,
            "actual doctor retirement expired"
        );
        if !private_eof {
            match owner.poll_notification(&cancel).unwrap() {
                LinuxControlRead::Frame(frame) => notifications.push(frame.notification),
                LinuxControlRead::ConfirmedEof => private_eof = true,
                LinuxControlRead::Pending => {}
            }
        }
        owner.read_stdout().unwrap();
        owner.read_stderr().unwrap();
        if owner.poll_retirement(deadline).unwrap() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(notifications.contains(&ControlNotification::CleanupComplete {}));
    assert_eq!(std::fs::read(&service_slot).unwrap(), b"DGSL01C\n");
    assert_eq!(std::fs::read(&bootstrap).unwrap(), b"DGSL01C\n");
}

/// 只复制CI本次构建镜像，root隔离夹具随后固定路径及权限；不建立产品签名信任。
fn copy_current_image(variable: &str, destination: &Path) {
    let source = std::env::var_os(variable).expect("explicit current build image required");
    let metadata = std::fs::metadata(&source).unwrap();
    assert!(metadata.is_file() && metadata.len() > 0 && metadata.len() <= 256 << 20);
    std::fs::copy(source, destination).unwrap();
    std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o555)).unwrap();
}

/// 用固定缓冲读取实际完整镜像摘要，不将文件大小用于分配。
fn file_digest(path: &Path) -> [u8; 32] {
    let mut image = File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut bytes = [0; 64 << 10];
    loop {
        let count = image.read(&mut bytes).unwrap();
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    digest.finalize().into()
}

/// 固定隔离服务身份持有数据及服务槽；不是授权任意前端。
fn chown_service(path: &Path) {
    let path = CString::new(path.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::chown(path.as_ptr(), 23111, 23111) }, 0);
}
