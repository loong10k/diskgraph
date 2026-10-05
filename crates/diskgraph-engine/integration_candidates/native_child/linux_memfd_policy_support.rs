//! 隔离策略夹具的有界真实进程宿主；不替代扫描器 Atomic 入口。

use super::linux_atomic_launcher_test_support::check;
use super::UnixChild;
use std::io::Read;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

const DEADLINE_ENV: &str = "DG_MEMFD_DEADLINE_NS";

/// 参数：无；返回：外层原单调截止的剩余窗口，跨 exec 不新建二十秒额度。
pub(super) fn original_deadline() -> Instant {
    let now = monotonic_ns();
    let end = match std::env::var(DEADLINE_ENV) {
        Ok(value) => value.parse::<u64>().unwrap(),
        Err(_) => now.checked_add(20_000_000_000).unwrap(),
    };
    let remaining = end
        .checked_sub(now)
        .expect("original host deadline expired");
    Instant::now()
        .checked_add(Duration::from_nanos(remaining))
        .unwrap()
}

/// 参数：无；返回：本案真实 proc mount 的策略读数，缺少读取能力直接资格失败。
pub(super) fn policy() -> u32 {
    let base = std::env::var_os("DG_MEMFD_PROC")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/proc"));
    let value = std::fs::read_to_string(base.join("sys/vm/memfd_noexec")).unwrap_or_else(|error| {
        panic!(
            "missing_qualification phase=read_memfd_noexec errno={:?} error={error}",
            error.raw_os_error()
        )
    });
    let value: u32 = value.trim().parse().unwrap();
    assert!(value <= 2, "unknown native policy {value}");
    value
}

/// 参数：scope 为目标策略，name 为精确测试名，body 为实际原生验收；返回：退出及阶段强断言。
pub(super) fn in_namespace(scope: u32, name: &str, body: impl FnOnce()) {
    if std::env::var("DG_MEMFD_POLICY_INNER").as_deref() == Ok(&scope.to_string()) {
        assert_eq!(
            unsafe { libc::getpid() },
            1,
            "actual new PID namespace leader"
        );
        if std::env::var_os("DG_MEMFD_PREPARED_OUTPUT").is_some() {
            require_prepared_runner();
        }
        assert_eq!(policy(), scope, "actual namespace sysctl readback");
        body();
        eprintln!("MEMFD_POLICY_CASE_COMPLETE scope={scope}");
        return;
    }
    let helper = std::env::var_os("DG_MEMFD_POLICY_NAMESPACE_FIXTURE")
        .map(PathBuf::from)
        .expect("root must compile and provide namespace fixture");
    assert!(helper.is_absolute() && helper.is_file());
    let directory = tempfile::tempdir().unwrap();
    let proc_mount = directory.path().join("proc");
    std::fs::create_dir(&proc_mount).unwrap();
    let mut command = Command::new(helper);
    command
        .arg(scope.to_string())
        .arg(proc_mount)
        .arg(std::env::current_exe().unwrap())
        .arg(name);
    let output = run_host(&mut command);
    assert!(
        output.contains("\"status\":\"namespace_qualified\""),
        "missing actual namespace identity witness: {output}"
    );
    assert!(
        output.contains(&format!("MEMFD_POLICY_CASE_COMPLETE scope={scope}")),
        "exact inner case did not complete: {output}"
    );
}

fn require_prepared_runner() {
    // 环境只传选择与原期限；实际 PID/proc/凭据/capabilities 必须逐项查询。
    assert_eq!(
        unsafe { libc::getpid() },
        1,
        "actual new PID namespace leader"
    );
    assert_eq!(
        std::fs::read_link("/proc/self").unwrap(),
        PathBuf::from("1")
    );
    assert_eq!(std::env::var("DG_MEMFD_PROC").unwrap(), "/proc");
    assert_eq!(
        std::env::var("DG_NATIVE_NAMESPACE_SUPERVISED").unwrap(),
        "1"
    );
    assert_eq!(
        std::fs::read_link("/proc/self/ns/pid").unwrap(),
        PathBuf::from(std::env::var("DG_NATIVE_PID_NAMESPACE").unwrap())
    );
    let uid: libc::uid_t = std::env::var("DG_MEMFD_RUNNER_UID")
        .unwrap()
        .parse()
        .unwrap();
    let gid: libc::gid_t = std::env::var("DG_MEMFD_RUNNER_GID")
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(uid, 0, "runner libtest must never execute with GLOBAL UID0");
    let (mut real_uid, mut effective_uid, mut saved_uid) = (0, 0, 0);
    let (mut real_gid, mut effective_gid, mut saved_gid) = (0, 0, 0);
    assert_eq!(
        unsafe { libc::getresuid(&mut real_uid, &mut effective_uid, &mut saved_uid) },
        0
    );
    assert_eq!(
        unsafe { libc::getresgid(&mut real_gid, &mut effective_gid, &mut saved_gid) },
        0
    );
    assert_eq!((real_uid, effective_uid, saved_uid), (uid, uid, uid));
    assert_eq!((real_gid, effective_gid, saved_gid), (gid, gid, gid));
    let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    assert!(
        (0..=1024).contains(&count),
        "bounded actual supplementary groups"
    );
    let mut actual_groups = vec![0; usize::try_from(count).unwrap()];
    assert_eq!(
        unsafe { libc::getgroups(count, actual_groups.as_mut_ptr()) },
        count
    );
    let mut expected_groups: Vec<libc::gid_t> = std::env::var("DG_MEMFD_RUNNER_GROUPS")
        .unwrap()
        .split(',')
        .map(|value| value.parse().unwrap())
        .collect();
    actual_groups.sort_unstable();
    expected_groups.sort_unstable();
    assert_eq!(actual_groups, expected_groups);
    let mut status = String::new();
    std::fs::File::open("/proc/self/status")
        .unwrap()
        .take((16 << 10) + 1)
        .read_to_string(&mut status)
        .unwrap();
    assert!(
        status.len() <= (16 << 10),
        "bounded actual credential status"
    );
    for key in [
        "CapInh",
        "CapPrm",
        "CapEff",
        "CapBnd",
        "CapAmb",
        "NoNewPrivs",
    ] {
        let fields: Vec<&str> = status
            .lines()
            .filter_map(|line| line.split_once(':'))
            .filter(|(name, _)| *name == key)
            .map(|(_, value)| value.trim())
            .collect();
        assert_eq!(fields.len(), 1, "unique actual credential field {key}");
        if key == "NoNewPrivs" {
            assert_eq!(fields[0], "1");
        } else {
            assert_eq!(
                u64::from_str_radix(fields[0], 16).unwrap(),
                0,
                "actual {key}"
            );
        }
    }
    let end: u64 = std::env::var(DEADLINE_ENV).unwrap().parse().unwrap();
    assert!(
        monotonic_ns() < end,
        "original pre-birth policy deadline remains live"
    );
}

/// 参数：name 为精确本测试入口，body 为局部 seccomp 案；返回：实际隔离宿主的结束见证。
pub(super) fn isolated(name: &str, body: impl FnOnce()) {
    if std::env::var("DG_MEMFD_DENIAL_INNER").as_deref() == Ok(name) {
        body();
        eprintln!("MEMFD_DENIAL_CASE_COMPLETE");
        return;
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env("DG_MEMFD_DENIAL_INNER", name);
    let output = run_host(&mut command);
    assert!(
        output.contains("MEMFD_DENIAL_CASE_COMPLETE"),
        "exact inner case did not complete: {output}"
    );
}

fn run_host(command: &mut Command) -> String {
    let absolute = monotonic_ns().checked_add(20_000_000_000).unwrap();
    let end = Instant::now()
        .checked_add(Duration::from_nanos(absolute - monotonic_ns()))
        .unwrap();
    command.env(DEADLINE_ENV, absolute.to_string());
    let mut child = UnixChild::spawn(command, || check(end)).unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let observed = catch_unwind(AssertUnwindSafe(|| {
        loop {
            check(end).unwrap();
            if let Some(bytes) = child.read_stdout().unwrap() {
                append(&mut out, bytes);
            }
            if let Some(bytes) = child.read_stderr().unwrap() {
                append(&mut err, bytes);
            }
            if child.poll().unwrap() && child.stdout_eof() && child.stderr_eof() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        child.exit_code()
    }));
    let cleanup = child.cleanup();
    let output = format!(
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(&err)
    );
    match observed {
        Ok(code) => {
            eprintln!("MEMFD_POLICY_HOST exit={code:?} cleanup={cleanup:?} {output}");
            cleanup.unwrap();
            assert_ne!(
                code,
                Some(77),
                "missing_qualification is not acceptance: {output}"
            );
            assert_eq!(code, Some(0), "actual fixture failure: {output}");
            output
        }
        Err(payload) => {
            eprintln!("MEMFD_POLICY_HOST failed cleanup={cleanup:?} {output}");
            resume_unwind(payload);
        }
    }
}

fn append(output: &mut Vec<u8>, bytes: &[u8]) {
    assert!(
        bytes.len() <= 4096 - output.len(),
        "bounded qualification stream"
    );
    output.extend_from_slice(bytes);
}

fn monotonic_ns() -> u64 {
    let mut time: libc::timespec = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) },
        0
    );
    u64::try_from(time.tv_sec)
        .unwrap()
        .checked_mul(1_000_000_000)
        .unwrap()
        .checked_add(u64::try_from(time.tv_nsec).unwrap())
        .unwrap()
}
