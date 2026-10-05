//! 真实机制探针的窄读与 actual wait；来源：原生 Rust Child 与明确 artifact。

use super::linux_atomic_birth_fixture::AtomicBirthFixture;
use super::{ChildError, UnixChild};
use serde_json::Value;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// 执行一个原生机制场景；参数：fixture/mode 为明确 artifact 与固定场景；返回：有界真实报告。
pub(super) fn run(fixture: &AtomicBirthFixture, mode: &str) -> Value {
    // 原 20s 从准备前只创建一次；两份镜像资格与实际机制场景共用，不刷新期限。
    let started = Instant::now();
    // 两份镜像先实际执行资格正控，不能只以路径不同声称不同 image。
    assert_eq!(image(fixture.binary(), started)["image"], 1);
    assert_eq!(image(fixture.replacement(), started)["image"], 2);
    let mut command = Command::new(fixture.binary());
    command
        .arg(mode)
        .arg(fixture.directory())
        .arg(fixture.replacement());
    let (status, stdout, stderr) = execute(&mut command, started);
    let report: Value = serde_json::from_slice(&stdout).unwrap();
    println!("DG_ATOMIC_BIRTH mode={mode} status={status:?} report={report}");
    assert!(stderr.is_empty(), "fixed native probe stderr {stderr:?}");
    assert!(
        status == Some(0),
        "native qualification not proven; exit 77 is not a capability pass: {report}"
    );
    assert_eq!(report["qualified"], true);
    assert_eq!(report["phase"], "complete");
    assert_eq!(report["errno"], 0);
    assert_eq!(report["pid_reuse_verified"], false);
    #[cfg(target_arch = "x86_64")]
    assert_eq!(report["abi"], "x86_64");
    #[cfg(target_arch = "aarch64")]
    assert_eq!(report["abi"], "aarch64");
    report
}

fn image(path: &Path, started: Instant) -> Value {
    let (status, stdout, stderr) = execute(Command::new(path).arg("image"), started);
    assert!(status == Some(0) && stderr.is_empty(), "qualified image");
    serde_json::from_slice(&stdout).unwrap()
}

fn execute(command: &mut Command, started: Instant) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    command.env_clear();
    let mut stdout = Vec::with_capacity(4096);
    let mut stderr = Vec::with_capacity(4096);
    // Null 旧探针不安装 scanner 派生过滤器，C 必须真实调用 clone；保留 leader 到组清理。
    let mut child = UnixChild::spawn(command, || check_window(started)).unwrap();
    let observed = catch_unwind(AssertUnwindSafe(|| -> Result<Option<i32>, ChildError> {
        loop {
            check_window(started)?;
            if let Some(bytes) = child.read_stdout()? {
                append_bounded(&mut stdout, bytes)?;
            }
            if let Some(bytes) = child.read_stderr()? {
                append_bounded(&mut stderr, bytes)?;
            }
            // WNOWAIT 只观察，不先消费 leader；异常 pipe holder 也受同一期限与组回收约束。
            let exited = child.poll()?;
            check_window(started)?;
            if exited && child.stdout_eof() && child.stderr_eof() {
                return Ok(child.exit_code());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }));
    let status = match observed {
        Ok(Ok(status)) => {
            // 即使收到成功 C 报告，也明确回收测试宿主组；不充 C 场景的正常 wait 许可。
            child.cleanup().unwrap();
            status
        }
        Ok(Err(error)) => {
            let failure = error.with_cleanup(child.cleanup());
            panic!("native mechanism host failed: {failure:?}");
        }
        Err(payload) => {
            let cleanup = child.cleanup();
            eprintln!("native mechanism host panic cleanup={cleanup:?}");
            resume_unwind(payload);
        }
    };
    (status, stdout, stderr)
}

fn check_window(started: Instant) -> Result<(), ChildError> {
    if started.elapsed() >= Duration::from_secs(20) {
        return Err(ChildError::Unsupported(
            "atomic birth fixture deadline exceeded",
        ));
    }
    Ok(())
}

fn append_bounded(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ChildError> {
    if bytes.len() > 4096 - output.len() {
        return Err(ChildError::Unsupported(
            "atomic birth fixture output exceeded",
        ));
    }
    output.extend_from_slice(bytes);
    Ok(())
}
