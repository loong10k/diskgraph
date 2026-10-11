use super::{ControlError, FrontendRead, LinuxFrontendConnection, UnixForegroundOutput};
use std::os::unix::{fs::PermissionsExt, net::UnixListener, process::CommandExt};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn run(mode: &str, uid: u32, expected_rejection: bool) {
    assert_eq!(
        unsafe { libc::getuid() },
        0,
        "isolated root qualification only"
    );
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let socket = directory.path().join("frontend");
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).unwrap();
    LinuxFrontendConnection::prepare_listener(&listener).unwrap();
    // client按真实UID执行；固定角色复制至本root拥有的0755夹具，不能依赖runner私有祖先可穿越。
    let role = directory.path().join("frontend_role");
    std::fs::copy(std::env::var_os("DG_LINUX_SUPERVISOR_ROLE").unwrap(), &role).unwrap();
    std::fs::set_permissions(&role, std::fs::Permissions::from_mode(0o555)).unwrap();
    let mut command = Command::new(role);
    command.args(["frontend", socket.to_str().unwrap(), mode]);
    unsafe {
        command.pre_exec(move || {
            if libc::setgroups(0, std::ptr::null()) != 0
                || libc::setgid(uid) != 0
                || libc::setuid(uid) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // 并行fork夹具可能短暂继承另一线程尚未关闭的写句柄，导致exec在业务出生前ETXTBSY。
    // 只对该原OS错误在固定准备期限内重试同一只读镜像；不重试协议、身份或运行失败。
    let birth_deadline = Instant::now() + Duration::from_secs(5);
    let mut transient_busy = 0;
    let child = loop {
        match command.stdout(std::process::Stdio::piped()).spawn() {
            Ok(child) => break child,
            Err(error)
                if error.raw_os_error() == Some(libc::ETXTBSY)
                    && Instant::now() < birth_deadline =>
            {
                transient_busy += 1;
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => {
                panic!("frontend pre-exec failed: {error:?}; transient_busy={transient_busy}")
            }
        }
    };
    eprintln!("frontend actual birth succeeded: transient_busy={transient_busy}");
    let (stream, _) = listener.accept().unwrap();
    let admitted = LinuxFrontendConnection::new(UnixForegroundOutput::new(stream).unwrap(), 23112);
    let deadline = Instant::now() + Duration::from_secs(5);
    let cancel = AtomicBool::new(false);
    if uid != 23112 {
        assert!(matches!(admitted, Err(ControlError::Protocol)));
    } else {
        let mut connection = admitted.unwrap();
        let outcome = loop {
            assert!(Instant::now() < deadline);
            match connection.poll_input_until(deadline, &cancel) {
                Ok(FrontendRead::Pending) => std::thread::sleep(Duration::from_millis(1)),
                Ok(FrontendRead::Data(bytes)) => break Ok(bytes.to_vec()),
                other => break Err(other.err()),
            }
        };
        if expected_rejection {
            assert_eq!(outcome, Err(Some(ControlError::Protocol)));
            assert_eq!(
                connection.poll_input_until(deadline, &cancel).unwrap_err(),
                ControlError::Protocol
            );
            assert!(connection.into_output().is_err());
        } else {
            assert_eq!(outcome.unwrap(), b"DG_FRONT_INPUT\n");
            loop {
                assert!(Instant::now() < deadline);
                match connection.poll_input_until(deadline, &cancel).unwrap() {
                    FrontendRead::Pending => std::thread::sleep(Duration::from_millis(1)),
                    FrontendRead::Eof => break,
                    other => panic!("unexpected {other:?}"),
                }
            }
            connection
                .write_output_until(b"DG_FRONT_OUTPUT", deadline, &cancel)
                .unwrap();
            connection.into_output().unwrap().finish().unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{mode}: {:?}", output.status);
    if mode == "rights" {
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["unexpected_fd_closed"],
            true
        );
    }
}

#[test]
#[ignore = "requires explicit compiled role and isolated root; invoked by native qualification"]
fn original_frontend_identity_and_half_close_keep_exact_output() {
    run("plain", 23112, false);
}
#[test]
#[ignore = "requires explicit compiled role and isolated root; invoked by native qualification"]
fn other_frontend_uid_is_rejected_before_input() {
    run("deny", 23113, true);
}
#[test]
#[ignore = "requires explicit compiled role and isolated root; invoked by native qualification"]
fn transferred_connection_does_not_inherit_original_sender_identity() {
    run("delegate", 23112, true);
}
#[test]
#[ignore = "requires explicit compiled role and isolated root; invoked by native qualification"]
fn unexpected_frontend_fd_is_closed_and_rejected() {
    run("rights", 23112, true);
}
