//! 独立程序启动的子进程回收策略；来源：RT-08/PF-06，无Java对等对象。

/// 为完全拥有进程的独立入口准备子进程退出通知。
/// 参数：无，必须早于任何线程、子进程或Engine出生；嵌入式宿主不得隐式调用。
/// 返回：策略准备成功或原系统错误；不提供原owner回收完成的证明。
pub fn prepare_standalone_child_reaping() -> std::io::Result<()> {
    // 该函数仅由尚未出生任何业务线程/子进程的独立入口调用。
    // 清空NOCLDWAIT并恢复默认处理，防止内核绕过原owner自动回收。
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = libc::SIG_DFL;
    if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::sigaction(libc::SIGCHLD, &action, std::ptr::null_mut()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::prepare_standalone_child_reaping;
    use std::process::{Command, Stdio};

    #[test]
    fn inherited_auto_reaping_is_normalized_in_an_isolated_real_process() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "standalone_child_reaping::tests::auto_reaping_fixture",
                "--ignored",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("DG_STANDALONE_CHILD_REAPED=3"));
    }

    #[test]
    #[ignore = "由真实隔离父用例显式调用，不改动测试父进程信号"]
    fn auto_reaping_fixture() {
        for (handler, flags) in [
            (libc::SIG_IGN, 0),
            (libc::SIG_DFL, libc::SA_NOCLDWAIT),
            (libc::SIG_IGN, libc::SA_NOCLDWAIT),
        ] {
            // 本夹具是独占测试子进程；没有其他业务线程或子进程。
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = handler;
            action.sa_flags = flags;
            assert_eq!(unsafe { libc::sigemptyset(&mut action.sa_mask) }, 0);
            assert_eq!(
                unsafe { libc::sigaction(libc::SIGCHLD, &action, std::ptr::null_mut()) },
                0
            );
            prepare_standalone_child_reaping().unwrap();
            let mut original = Command::new(std::env::current_exe().unwrap())
                .arg("--help")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            assert!(
                original
                    .wait()
                    .expect("original child must remain waitable")
                    .success()
            );
        }
        println!("DG_STANDALONE_CHILD_REAPED=3");
    }
}
