//! 仅独立进程验证Darwin永久身份切换；不安装服务或创建系统账号。
#[cfg(target_os = "macos")]
fn main() {
    use diskgraph_engine::supervisor_service::MacosGuardianIdentity;
    let mode = std::env::args()
        .nth(1)
        .expect("explicit fixture mode required");
    let original = unsafe {
        (
            libc::getuid(),
            libc::geteuid(),
            libc::getgid(),
            libc::getegid(),
        )
    };
    match mode.as_str() {
        "invalid" => {
            for (uid, gid) in [(0, 23113), (23111, 0), (u32::MAX, 23113), (23111, u32::MAX)] {
                let error = unsafe { MacosGuardianIdentity::enter(uid, gid) }.unwrap_err();
                assert_eq!(error.raw_os_error(), Some(libc::EINVAL));
                assert_eq!(
                    unsafe {
                        (
                            libc::getuid(),
                            libc::geteuid(),
                            libc::getgid(),
                            libc::getegid(),
                        )
                    },
                    original
                );
            }
            println!("DG_MACOS_GUARDIAN_INVALID_UNCHANGED=1");
        }
        "nonroot" => {
            assert_ne!(original.0, 0);
            assert_ne!(original.1, 0);
            let error = unsafe { MacosGuardianIdentity::enter(23111, 23113) }.unwrap_err();
            assert_eq!(error.raw_os_error(), Some(libc::EPERM));
            assert_eq!(
                unsafe {
                    (
                        libc::getuid(),
                        libc::geteuid(),
                        libc::getgid(),
                        libc::getegid(),
                    )
                },
                original
            );
            println!("DG_MACOS_GUARDIAN_NONROOT_UNCHANGED=1");
        }
        "drop" => {
            assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
            assert_eq!(
                std::env::var("RUNNER_ENVIRONMENT").as_deref(),
                Ok("github-hosted")
            );
            assert_eq!(
                std::env::var("DG_ISOLATED_ROOT_ACCEPTANCE").as_deref(),
                Ok("1")
            );
            assert_eq!(original.0, 0);
            assert_eq!(original.1, 0);
            // main未建立线程或Engine；本原进程独占承担身份切换，不影响CI宿主。
            unsafe { MacosGuardianIdentity::enter(23111, 23113) }.unwrap();
            assert_eq!(unsafe { libc::setuid(0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EPERM)
            );
            assert_eq!(unsafe { libc::setegid(0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EPERM)
            );
            println!("DG_MACOS_GUARDIAN_PERMANENT_IDENTITY=1");
        }
        _ => panic!("unsupported fixture mode"),
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Darwin identity fixture requires macOS");
    std::process::exit(10);
}
