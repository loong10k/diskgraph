use super::guardian_frontend_binding::GuardianFrontendBinding;
use super::linux_guardian_frontend::{ROOT_HELLO, authenticate};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

fn connection() -> (UnixStream, UnixStream) {
    let (client, server) = UnixStream::pair().unwrap();
    let enabled: libc::c_int = 1;
    assert_eq!(
        unsafe {
            libc::setsockopt(
                client.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const libc::c_int).cast(),
                std::mem::size_of_val(&enabled) as _,
            )
        },
        0
    );
    (client, server)
}

fn binding() -> GuardianFrontendBinding {
    GuardianFrontendBinding::decode(br#"{"schema_version":1,"socket_path":"/var/lib/dg/front.sock","frontend_uid":23112,"guardian_uid":23111,"guardian_gid":23113}"#,
        "/var/lib/dg/front.sock").unwrap()
}

#[test]
#[ignore = "isolated Linux root SCM_CREDENTIALS qualification"]
fn original_root_greeting_binds_same_pid_to_installed_guardian_identity() {
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let (mut client, mut server) = connection();
    server.write_all(ROOT_HELLO).unwrap();
    let expected = authenticate(
        &mut client,
        &binding(),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
    let mut actual = libc::ucred {
        pid: std::process::id() as _,
        uid: 23111,
        gid: 23113,
    };
    assert!(expected.matches(&actual));
    actual.pid += 1;
    assert!(!expected.matches(&actual));
    actual.pid -= 1;
    actual.uid = 0;
    assert!(!expected.matches(&actual));
    actual.uid = 23111;
    actual.gid = 23112;
    assert!(!expected.matches(&actual));
}

#[test]
#[ignore = "isolated Linux root SCM_CREDENTIALS qualification"]
fn malformed_or_truncated_original_root_greeting_is_refused() {
    assert_eq!(unsafe { libc::geteuid() }, 0);
    for bytes in [b"DGGU00R\n".as_slice(), b"DGG".as_slice()] {
        let (mut client, mut server) = connection();
        server.write_all(bytes).unwrap();
        drop(server);
        assert!(
            authenticate(
                &mut client,
                &binding(),
                Instant::now() + Duration::from_secs(2)
            )
            .is_err()
        );
    }
}

#[test]
fn actual_nonroot_sender_cannot_supply_root_greeting() {
    // root资格由独立ignored门禁执行；普通workspace只在实际非root身份下验证此负例。
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let (mut client, mut server) = connection();
    server.write_all(ROOT_HELLO).unwrap();
    assert!(
        authenticate(
            &mut client,
            &binding(),
            Instant::now() + Duration::from_secs(2)
        )
        .is_err()
    );
}

#[test]
#[ignore = "isolated Linux root original manifest qualification"]
fn original_public_binding_refuses_symlinks_foreign_owner_and_oversize_without_fallback() {
    use super::linux_guardian_frontend::installed_binding;
    use std::os::unix::fs::symlink;
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let domain = tempfile::Builder::new()
        .prefix("dg-front-binding-")
        .tempdir_in("/var/lib")
        .unwrap();
    let socket = domain.path().join("front.sock");
    let manifest = domain.path().join("guardian-frontend.json");
    let deadline = Instant::now() + Duration::from_secs(3);
    assert!(installed_binding(&socket, deadline).unwrap().is_none());
    let body = serde_json::json!({"schema_version":1,"socket_path":socket.to_str().unwrap(),
        "frontend_uid":23112,"guardian_uid":23111,"guardian_gid":23113});
    std::fs::write(&manifest, serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(installed_binding(&socket, deadline).unwrap().is_some());
    let original = std::fs::File::open(&manifest).unwrap();
    assert_eq!(
        unsafe { libc::fchown(original.as_raw_fd(), 23112, 23112) },
        0
    );
    assert!(installed_binding(&socket, deadline).is_err());
    assert_eq!(unsafe { libc::fchown(original.as_raw_fd(), 0, 0) }, 0);
    std::fs::write(&manifest, vec![b' '; 4097]).unwrap();
    assert!(installed_binding(&socket, deadline).is_err());
    std::fs::remove_file(&manifest).unwrap();
    let target = domain.path().join("real-manifest");
    std::fs::write(&target, serde_json::to_vec(&body).unwrap()).unwrap();
    symlink(&target, &manifest).unwrap();
    assert!(installed_binding(&socket, deadline).is_err());
}
