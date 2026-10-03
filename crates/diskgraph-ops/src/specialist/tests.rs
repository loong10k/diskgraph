use crate::OpsError;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;
use std::io::Write;

/// 由独立 Rust 测试进程写入可执行脚本，退出后返回夹具路径。
/// 参数：directory/name 为隔离路径，body 为受控脚本正文；返回：已关闭的原路径。
fn fake_tool(directory: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = directory.join(name);
    // 父测试进程不打开写句柄，避免其他并行 fork 暂时继承脚本 writer。
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "specialist::tests::script_writer_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env_clear()
        .env("DG_SPECIALIST_SCRIPT_PATH", &path)
        .env("DG_SPECIALIST_SCRIPT_BODY", body)
        .output()
        .unwrap();
    assert!(output.status.success(), "script writer failed: {output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("test result: ok. 1 passed; 0 failed;")
            && stdout.contains("DG_SPECIALIST_SCRIPT_WRITTEN"),
        "script writer did not confirm completion: {stdout}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        format!("#!/bin/sh\n{body}").as_bytes()
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o755
    );
    path
}

// 此 helper 不作为独立验收；fake_tool 显式调用并检查退出、实际运行数与完整产物。
#[test]
#[ignore = "独立脚本写入子进程；由 fake_tool 显式调用"]
fn script_writer_fixture() {
    use std::os::unix::fs::PermissionsExt;

    let path = std::env::var_os("DG_SPECIALIST_SCRIPT_PATH")
        .expect("script writer requires its controlled destination");
    let body = std::env::var("DG_SPECIALIST_SCRIPT_BODY")
        .expect("script writer requires its controlled body");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(file, "#!/bin/sh").unwrap();
    write!(file, "{body}").unwrap();
    file.flush().unwrap();
    file.sync_all().unwrap();
    drop(file);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    println!("DG_SPECIALIST_SCRIPT_WRITTEN");
}

fn spec_for(program: PathBuf, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program,
        args: args.iter().map(|argument| argument.to_string()).collect(),
        cwd: None,
        env: SandboxedRunner::default_env(),
        timeout_ms: 10_000,
        max_output_bytes: 1 << 20,
        retries: 0,
    }
}

#[test]
fn an_unlisted_capability_is_refused_even_when_known() {
    let registry = AdapterRegistry::empty();
    assert!(matches!(
        registry.capability("cargo-clean"),
        Err(OpsError::NotAuthorized(message)) if message.contains("allow-list")
    ));
    // An id this build has never heard of is refused for a different
    // reason, and equally offers nothing.
    assert!(registry.capability("rm-everything").is_err());
}

#[test]
fn a_missing_tool_reports_unavailable_and_offers_nothing() {
    let program = PathBuf::from("/nonexistent/cargo");
    let registry = AdapterRegistry::allowing(&[("cargo-clean", program.clone())]);
    let (capability, resolved) = registry.capability("cargo-clean").unwrap();
    assert_eq!(resolved, program.as_path());
    match probe(capability, resolved, &SandboxedRunner) {
        AdapterStatus::Unavailable { reason } => {
            assert!(reason.contains("unavailable"), "{reason}");
        }
        other => panic!("expected unavailable, got {other:?}"),
    }
}

#[test]
fn a_probe_accepts_a_new_enough_version_and_refuses_an_old_one() {
    let workspace = tempfile::TempDir::with_prefix("dg-specialist-probe-").unwrap();
    let new_tool = fake_tool(workspace.path(), "cargo-new", "echo \"fake-tool 1.99.0\"\n");
    let old_tool = fake_tool(workspace.path(), "cargo-old", "echo \"fake-tool 0.9.1\"\n");
    let (capability, _) = AdapterRegistry::allowing(&[("cargo-clean", new_tool.clone())])
        .capability("cargo-clean")
        .unwrap();
    // 只执行一次真实探针；失败时保留该次运行的状态，区分启动、期限与版本错误。
    let new_status = probe(capability, &new_tool, &SandboxedRunner);
    assert!(
        matches!(&new_status, AdapterStatus::Available { version } if version.contains("1.99.0")),
        "new-enough specialist fixture returned {new_status:?}"
    );
    let old_status = probe(capability, &old_tool, &SandboxedRunner);
    assert!(
        matches!(&old_status, AdapterStatus::Unavailable { reason } if reason.contains("below the required")),
        "old specialist fixture returned {old_status:?}"
    );
}

#[test]
fn structured_arguments_pass_through_without_a_shell() {
    // The payload would execute under a shell; argv passthrough must
    // deliver it as one literal argument instead (EC-04).
    let payload = "x;$(whoami)`rm -rf /`|deadly";
    let outcome = SandboxedRunner
        .run(&spec_for(
            PathBuf::from("/usr/bin/printf"),
            &["%s\\n", payload],
        ))
        .unwrap();
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.stdout, format!("{payload}\n").into_bytes());
}

#[test]
fn a_child_sees_only_the_named_environment() {
    let mut spec = spec_for(PathBuf::from("/usr/bin/env"), &[]);
    spec.env = vec![("PATH".into(), "/usr/bin:/bin".into())];
    let outcome = SandboxedRunner.run(&spec).unwrap();
    let printed = String::from_utf8_lossy(&outcome.stdout);
    assert!(printed.contains("PATH="));
    assert!(
        !printed.contains("HOME="),
        "the caller's whole environment leaked into the child"
    );
}

#[test]
fn a_timeout_kills_a_stuck_child() {
    let started = Instant::now();
    let mut spec = spec_for(PathBuf::from("/bin/sleep"), &["30"]);
    spec.timeout_ms = 200;
    let outcome = SandboxedRunner.run(&spec).unwrap();
    assert!(outcome.timed_out);
    assert_eq!(outcome.exit_code, -1);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[cfg(unix)]
#[test]
fn timeout_kills_descendants_that_inherit_output_pipes() {
    let started = Instant::now();
    let mut spec = spec_for(PathBuf::from("/bin/sh"), &["-c", "sleep 2 & wait"]);
    spec.timeout_ms = 50;
    let outcome = SandboxedRunner.run(&spec).unwrap();
    assert!(outcome.timed_out);
    assert!(
        started.elapsed() < Duration::from_millis(800),
        "descendant kept the reader threads alive after timeout"
    );
}

#[cfg(unix)]
#[test]
fn timeout_kills_a_descendant_after_the_direct_child_exits() {
    let workspace = tempfile::tempdir().unwrap();
    let marker = workspace.path().join("escaped");
    let script = format!(
        "(/bin/sleep 1; /usr/bin/touch {}) & exit 0",
        marker.display()
    );
    let mut spec = spec_for(PathBuf::from("/bin/sh"), &["-c", &script]);
    spec.timeout_ms = 50;
    let outcome = SandboxedRunner.run(&spec).unwrap();
    assert!(outcome.timed_out);
    std::thread::sleep(Duration::from_millis(1_200));
    assert!(!marker.exists(), "a descendant survived the timeout");
}

#[test]
fn output_past_the_cap_is_discarded_and_flagged() {
    let workspace = tempfile::TempDir::with_prefix("dg-specialist-cap-").unwrap();
    let chatty = fake_tool(workspace.path(), "chatty", "head -c 100000 /dev/zero\n");
    let mut spec = spec_for(chatty, &[]);
    spec.max_output_bytes = 1_024;
    let outcome = SandboxedRunner.run(&spec).unwrap();
    assert!(outcome.truncated);
    assert!(outcome.stdout.len() <= 1_024);
}

#[test]
fn retries_rerun_a_transient_failure() {
    let workspace = tempfile::TempDir::with_prefix("dg-specialist-retry-").unwrap();
    let counter = workspace.path().join("count");
    let flaky = fake_tool(
        workspace.path(),
        "flaky",
        &format!(
            "n=$(cat {} 2>/dev/null || echo 0); n=$((n+1)); echo $n > {}; [ $n -ge 3 ]\n",
            counter.display(),
            counter.display()
        ),
    );
    let mut spec = spec_for(flaky, &[]);
    spec.retries = 3;
    let outcome = SandboxedRunner.run(&spec).unwrap();
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(std::fs::read_to_string(&counter).unwrap().trim(), "3");
}

#[test]
fn only_a_provable_outcome_counts_as_confirmed() {
    let good = RunOutcome {
        exit_code: 0,
        stdout: b"Deleted volumes: vol-1".to_vec(),
        stderr: Vec::new(),
        timed_out: false,
        truncated: false,
    };
    assert_eq!(
        verify_specialist_result(&good, "Deleted volumes"),
        SpecialistVerdict::Confirmed
    );
    let mut failed = good.clone();
    failed.exit_code = 1;
    assert!(matches!(
        verify_specialist_result(&failed, "Deleted volumes"),
        SpecialistVerdict::NeedsAttention { .. }
    ));
    let mut cut = good.clone();
    cut.truncated = true;
    assert!(matches!(
        verify_specialist_result(&cut, "Deleted volumes"),
        SpecialistVerdict::NeedsAttention { reason } if reason.contains("truncated")
    ));
    let mut silent = good;
    silent.stdout.clear();
    assert!(matches!(
        verify_specialist_result(&silent, "Deleted volumes"),
        SpecialistVerdict::NeedsAttention { .. }
    ));
}

#[test]
fn an_active_build_is_detected_by_the_held_lock_not_by_the_file() {
    let workspace = tempfile::TempDir::with_prefix("dg-specialist-cargo-").unwrap();
    let project = workspace.path().join("proj");
    std::fs::create_dir_all(project.join("target/debug")).unwrap();
    std::fs::write(project.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(project.join("target/debug/blob"), vec![0_u8; 4096]).unwrap();
    std::fs::write(project.join("target/debug/.cargo-lock"), b"").unwrap();

    // The lock file persists after a build, so an idle target is idle
    // even with the file present.
    let idle = cargo_inventory(&project).unwrap();
    assert!(
        !idle.active_build,
        "an uncontended lock file is not a build"
    );

    // Holding the advisory lock is the real in-progress signal.
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let held = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(project.join("target/debug/.cargo-lock"))
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        let busy = cargo_inventory(&project).unwrap();
        assert!(busy.active_build, "a held lock marks a live build");
        assert!(busy.notes.iter().any(|note| note.contains("wait")));
        unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_UN) };
    }

    let inventory = cargo_inventory(&project).unwrap();
    assert_eq!(inventory.objects.len(), 1);
    assert_eq!(inventory.objects[0].kind, "cargo-target");
    assert_eq!(inventory.objects[0].path, project.join("target"));
    assert!(inventory.objects[0].bytes >= 4096);
}

#[test]
fn cargo_inventory_refuses_a_directory_that_is_not_a_cargo_project() {
    let workspace = tempfile::TempDir::with_prefix("dg-specialist-nocargo-").unwrap();
    assert!(matches!(
        cargo_inventory(workspace.path()),
        Err(OpsError::Stale(message)) if message.contains("Cargo.toml")
    ));
}

#[test]
fn a_shared_cargo_target_dir_is_reported_but_never_planned() {
    let workspace = tempfile::TempDir::with_prefix("dg-specialist-shared-").unwrap();
    let project = workspace.path().join("proj");
    std::fs::create_dir_all(project.join("target")).unwrap();
    std::fs::write(project.join("Cargo.toml"), "[package]\n").unwrap();
    let shared = workspace.path().join("shared-target");
    std::fs::create_dir_all(&shared).unwrap();
    let inventory = cargo_inventory_with_env(&project, Some(&shared.to_string_lossy())).unwrap();
    assert_eq!(inventory.objects.len(), 1);
    assert_eq!(inventory.objects[0].path, project.join("target"));
    assert!(
        inventory
            .notes
            .iter()
            .any(|note| note.contains("never planned")),
        "a shared build directory is a note, never an object"
    );
}
