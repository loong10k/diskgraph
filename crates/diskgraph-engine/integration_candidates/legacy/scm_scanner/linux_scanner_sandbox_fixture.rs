//! 隔离宿主的过滤器安装失败正控；来源：真实 Linux seccomp 拒绝与 std Child 回收。

use super::linux_scanner_sandbox_test_support as fixture;
use super::{ChildSpawnError, UnixChild};
use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;

#[test]
fn filter_denied_host() {
    let directory = match std::env::var_os("DG_SCANNER_FILTER_DENIED_HOST") {
        Some(directory) => PathBuf::from(directory),
        None => return,
    };
    // 外层 C 宿主已经装入仅拒后续安装的真实过滤器；这里调用实际生产启动 API。
    let mut calls = 0;
    let error = match UnixChild::spawn_scanner(
        &fixture::binary(),
        &[OsStr::new("report"), directory.as_os_str()],
        &[],
        || {
            calls += 1;
            Ok::<(), ()>(())
        },
    ) {
        Ok(_) => panic!("inner filter installation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(calls, 1, "failure precedes post-spawn checkpoint");
    assert!(
        matches!(&error, ChildSpawnError::Operation(native)
        if native.native_io_error().is_some_and(|error|
            error.kind() == std::io::ErrorKind::PermissionDenied
            && error.raw_os_error() == Some(libc::EPERM))),
        "{error:?}"
    );
    assert!(!directory.join("ready").exists(), "fixture never exec'd");
    assert!(
        !directory.join("pid").exists(),
        "no returned owner/publication fact"
    );
    let mut status = 0;
    // 仅此独立 --exact 宿主没有其它 std Child；标准 spawn 失败也必须回收其 child。
    assert_eq!(unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) }, -1);
    assert_eq!(
        io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
    println!("DG_INNER_FILTER_DENIED=EPERM_NO_EXEC_NO_CHILD");
}
