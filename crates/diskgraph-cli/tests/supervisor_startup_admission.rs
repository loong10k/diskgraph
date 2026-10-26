//! 真实 CLI 出生前监督准入回归；来源：PF-06 / frontend_recovery_supervisor。
//! 当前缺口的行为 RED，不以镜像缺失或解析失败代替未确认槽拒绝。

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a root-provisioned protected deployment fixture and actual scan image"]
fn unconfirmed_supervisor_slot_refuses_cli_before_database_birth() {
    use diskgraph_engine::recovery_slot::SlotReservation;
    use std::fs::File;
    use std::os::unix::fs::MetadataExt;
    use std::process::Command;
    use std::time::{Duration, Instant};

    let fixture = tempfile::tempdir().unwrap();
    let image = std::path::PathBuf::from(
        std::env::var_os("DISKGRAPH_FIXTURE_SCAN_IMAGE").expect("actual scan image"),
    );
    let digest =
        std::env::var_os("DISKGRAPH_FIXTURE_SCAN_SHA256").expect("independent fixture digest");
    let length = std::env::var_os("DISKGRAPH_FIXTURE_SCAN_BYTES").expect("actual image size");
    let state = std::path::PathBuf::from(
        std::env::var_os("DISKGRAPH_FIXTURE_SUPERVISOR_STATE").expect("root-provisioned state"),
    );
    let metadata = state.metadata().unwrap();
    assert!(state.is_absolute() && metadata.is_dir());
    assert_eq!(metadata.uid(), 0);
    assert_eq!(metadata.mode() & 0o022, 0);
    let slot = state.join(format!(
        "uid_{}.slot",
        fixture.path().metadata().unwrap().uid()
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    let active = SlotReservation::acquire(
        File::options().read(true).write(true).open(&slot).unwrap(),
        deadline,
    )
    .unwrap()
    .activate(deadline)
    .unwrap();
    // 真实持久 ACTIVE 在原锁释放后仍不可重用；不是文件存在或假 Pending 标志。
    drop(active);
    let original = std::fs::read(&slot).unwrap();
    assert_eq!(original, b"DGSL01A\n");
    let invoke = |data: &std::path::Path, with_state: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_diskgraph"));
        command
            .arg("--data-dir")
            .arg(data)
            .args(["--json", "doctor"]);
        command
            .env("DISKGRAPH_SCAN_WORKER_PATH", &image)
            .env("DISKGRAPH_SCAN_WORKER_SHA256", &digest)
            .env("DISKGRAPH_SCAN_WORKER_BYTES", &length)
            .env_remove("DISKGRAPH_SUPERVISOR_STATE_DIR");
        if with_state {
            command.env("DISKGRAPH_SUPERVISOR_STATE_DIR", &state);
        }
        bounded_output(command)
    };
    let baseline_data = fixture.path().join("baseline_data");
    let baseline = invoke(&baseline_data, false);
    assert!(
        baseline.status.success(),
        "baseline startup failed: {baseline:?}"
    );
    assert!(baseline_data.is_dir());
    assert!(baseline_data.join("diskgraph.sqlite").is_file());
    assert!(baseline_data.join("diskgraph-control.sqlite").is_file());
    let data = fixture.path().join("refused_data");
    let result = invoke(&data, true);
    let database_born = data.exists();
    let slot_after = std::fs::read(&slot).unwrap();
    assert!(
        !result.status.success() && !database_born && slot_after == original,
        "unconfirmed supervisor was ignored: database_born={database_born}, slot_unchanged={}, output={result:?}",
        slot_after == original,
    );
    let reply: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "recovery_unconfirmed");
}

/// 有限观察真实 CLI。参数：独立命令；返回：实际退出及有限输出，超时失败不计作产品恢复。
#[cfg(target_os = "linux")]
fn bounded_output(mut command: std::process::Command) -> std::process::Output {
    use std::io::{Read, Seek, SeekFrom};
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let mut stdout = tempfile::tempfile().unwrap();
    let mut stderr = tempfile::tempfile().unwrap();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.try_clone().unwrap()))
        .stderr(Stdio::from(stderr.try_clone().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut child = command.spawn().unwrap();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        let within_output = stdout
            .metadata()
            .unwrap()
            .len()
            .checked_add(stderr.metadata().unwrap().len())
            .is_some_and(|bytes| bytes <= 128 << 10);
        if Instant::now() >= deadline || !within_output {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "CLI admission observation exceeded its time/output budget; this is not cleanup confirmation"
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let read_output = |file: &mut std::fs::File| {
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.take((128 << 10) + 1).read_to_end(&mut bytes).unwrap();
        assert!(bytes.len() <= 128 << 10);
        bytes
    };
    let stdout = read_output(&mut stdout);
    let stderr = read_output(&mut stderr);
    assert!(stdout.len() + stderr.len() <= 128 << 10);
    std::process::Output {
        status,
        stdout,
        stderr,
    }
}
