//! 实际 Git 采样的出生时点诊断；不改变原预算、执行路径或原 owner 清理责任。
use super::windows_birth_test_hook::WindowsBirthTestHook;
use crate::ProbeHost;
use crate::live_evidence::{NativeEvidenceTestSession, ProbeLimits, run_managed_probe_for_test};
use std::cell::RefCell;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::time::Instant;

#[test]
#[ignore = "isolated native Git timing diagnostic, not production acceptance"]
fn actual_unborn_and_committed_git_child_birth_timings() {
    let (host, recovery) = ProbeHost::new(1).unwrap();
    let version_command = || {
        let mut command = Command::new("git");
        command.arg("--version").env_clear();
        for key in ["PATH", "SystemRoot"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
    };
    for iteration in 0..4 {
        let direct_started = Instant::now();
        let direct = version_command().output().unwrap();
        let direct_us = direct_started.elapsed().as_micros();
        assert!(direct.status.success());
        assert!(direct.stdout.starts_with(b"git version "));
        let native_started = Instant::now();
        let native =
            run_managed_probe_for_test(&mut version_command(), &ProbeLimits::default(), &host);
        println!(
            "DG_GIT_VERSION_CONTROL iteration={iteration} direct_us={direct_us} managed_us={} managed_success={}",
            native_started.elapsed().as_micros(),
            native.is_ok()
        );
        // 真实原owner确认退休后才进入下一轮；外层受控Job负责诊断宿主的最终边界。
        while !recovery.drain().unwrap() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        native.expect("managed Git version control must actually complete");
    }
    let directory = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "user.name=fixture", "-c", "user.email=f@example"])
            .args(args)
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "fixture setup: {output:?}");
    };
    git(&["init", "-q"]);
    git(&["symbolic-ref", "HEAD", "refs/heads/main"]);
    let mut results = Vec::new();
    for committed in [false, true] {
        if committed {
            std::fs::write(directory.path().join("a.txt"), b"one\n").unwrap();
            git(&["add", "."]);
            git(&["commit", "-q", "-m", "one"]);
        }
        let started = Instant::now();
        let births = Rc::new(RefCell::new(Vec::with_capacity(64)));
        let observed_births = Rc::clone(&births);
        let hook = WindowsBirthTestHook::install(move |_| {
            // 只记录真实 CreateProcess 后的时点；不读取额外句柄，不在原预算中输出日志。
            let mut observations = observed_births.borrow_mut();
            if observations.len() < 64 {
                observations.push(started.elapsed().as_micros());
            }
        });
        let mut session = NativeEvidenceTestSession::new(&ProbeLimits::default()).unwrap();
        let result = session.sample_git(Path::new("git"), directory.path());
        let sampled_us = started.elapsed().as_micros();
        drop(hook);
        println!(
            "DG_GIT_NATIVE_TIMING committed={committed} sampled_us={sampled_us} births_us={:?} success={}",
            births.borrow(),
            result.is_ok()
        );
        // 继续使用原测试宿主的实际恢复；恢复耗时单列，不冒充采样预算内完成。
        drop(session);
        println!(
            "DG_GIT_NATIVE_RECOVERY committed={committed} total_us={}",
            started.elapsed().as_micros()
        );
        results.push(result);
    }
    for result in results {
        result.expect("complete native Git sample required; failure retained as diagnostic");
    }
}
