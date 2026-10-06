use crate::macos_process_limit::MacosProcessLimit;
use std::process::Command;

#[test]
fn isolated_limit_fixture() {
    if std::env::var_os("DG_MACOS_PROCESS_LIMIT_FIXTURE").is_none() {
        return;
    }
    assert_ne!(unsafe { libc::getuid() }, 0);
    assert_ne!(unsafe { libc::geteuid() }, 0);
    MacosProcessLimit::install().unwrap();
    MacosProcessLimit::install().unwrap();
    let mut limits = libc::rlimit {
        rlim_cur: 9,
        rlim_max: 9,
    };
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NPROC, &mut limits) },
        0
    );
    assert_eq!((limits.rlim_cur, limits.rlim_max), (0, 0));
    assert_eq!(std::thread::spawn(|| 17).join().unwrap(), 17);
    let forked = unsafe { libc::fork() };
    let fork_error = std::io::Error::last_os_error().raw_os_error();
    if forked == 0 {
        unsafe { libc::_exit(0) };
    }
    if forked > 0 {
        loop {
            let waited = unsafe { libc::waitpid(forked, std::ptr::null_mut(), 0) };
            if waited == forked {
                break;
            }
            let error = std::io::Error::last_os_error();
            assert_eq!(
                error.raw_os_error(),
                Some(libc::EINTR),
                "unexpected wait failure"
            );
        }
        panic!("hard zero unexpectedly allowed fork; original child was reaped");
    }
    assert_eq!(forked, -1);
    assert_eq!(fork_error, Some(libc::EAGAIN));
    let error = match Command::new("/usr/bin/true").spawn() {
        Err(error) => error,
        Ok(mut child) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("hard zero must forbid new process");
        }
    };
    assert_eq!(error.raw_os_error(), Some(libc::EAGAIN));
    let raised = libc::rlimit {
        rlim_cur: 1,
        rlim_max: 1,
    };
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NPROC, &raised) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM)
    );
}

#[test]
fn helper_limit_is_real_and_does_not_change_parent_limit() {
    assert!(Command::new("/usr/bin/true").status().unwrap().success());
    let mut before = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NPROC, &mut before) },
        0
    );
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "macos_process_limit_tests::isolated_limit_fixture",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("DG_MACOS_PROCESS_LIMIT_FIXTURE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut after = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NPROC, &mut after) },
        0
    );
    assert_eq!(
        (before.rlim_cur, before.rlim_max),
        (after.rlim_cur, after.rlim_max)
    );
}
