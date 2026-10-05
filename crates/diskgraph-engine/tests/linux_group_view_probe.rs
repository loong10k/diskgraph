#![cfg(target_os = "linux")]
//! Linux 资格探针，不是生产 group-empty 许可；来源：真实 pthread/procfs/namespace 原语。

mod linux_group_view_probe {
    pub(super) mod child;
}

use linux_group_view_probe::child::{ProbeChild, pidfd_ready};
use serde_json::Value;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

#[test]
fn zombie_main_does_not_mean_the_thread_group_has_exited() {
    let mut process = ProbeChild::spawn("leader_exit");
    process.wait_file("ready");
    let pid = process.id();
    assert_eq!(process.scalar("pid"), u64::from(pid));
    assert_eq!(process.scalar("main_tid"), u64::from(pid));
    assert_eq!(process.scalar("pgid"), u64::from(pid));
    assert_eq!(process.scalar("sid"), u64::from(pid));
    let started = Instant::now();
    let threads = loop {
        let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let end = text.rfind(')').unwrap();
        let fields: Vec<_> = text[end + 2..].split_ascii_whitespace().collect();
        assert!(fields.len() > 19, "complete stat witness");
        assert_eq!(fields[2].parse::<u32>().unwrap(), pid, "same group");
        assert_eq!(fields[3].parse::<u32>().unwrap(), pid, "same session");
        if fields[0] == "Z" {
            break fields[17].parse::<u32>().unwrap();
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "main pthread_exit not reached"
        );
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(threads >= 2, "two original live workers remain");
    let first = process.advancing(0);
    let second = process.advancing(1);
    // 安全性：仅对该实际 child 创建 pidfd；未用数字列表宣称整个组为空。
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    assert!(
        raw >= 0,
        "pidfd required for this whole-thread-group qualification: {}",
        std::io::Error::last_os_error()
    );
    let pidfd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
    assert!(
        !pidfd_ready(pidfd.as_raw_fd()).unwrap(),
        "main Z still has live threads"
    );
    assert!(process.advancing(0) > first && process.advancing(1) > second);
    process.release();
    let started = Instant::now();
    while !pidfd_ready(pidfd.as_raw_fd()).unwrap() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "natural full group exit"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(process.wait().success());
    assert_eq!(process.scalar("finished_0"), 1);
    assert_eq!(process.scalar("finished_1"), 1);
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
    println!(
        "DG_LINUX_GROUP_PROBE={{\"mode\":\"leader_exit\",\"main_state\":\"Z\",\"live_threads\":{threads},\"heartbeats\":true,\"pidfd_live_before\":true,\"pidfd_exit_after_release\":true,\"actual_wait\":true}}"
    );
}

#[test]
fn current_procfs_reports_actual_namespace_and_mount_facts() {
    let records = run_view("view").expect("ordinary held procfs probe is required");
    let view = &records[0];
    assert_eq!(view["view"], "current");
    assert_eq!(view["nspid_last"], view["pid"]);
    assert_eq!(view["root_mount_id"], view["status_mount_id"]);
    assert_eq!(view["mount_record_found"], true);
    assert!(view["nspid_count"].as_u64().unwrap() >= 1);
    // 此处只保实际事实；filtered/ancestor 不会被转换为完整可见性许可。
}

#[test]
fn private_procfs_distinguishes_ancestor_from_current_pid_namespace() {
    let Some(records) = run_view("namespace") else {
        return;
    };
    assert_ancestor_and_current(&records);
    assert_eq!(records[1]["filtered"], false);
}

#[test]
fn actual_hidepid_mount_is_a_visibility_disqualification() {
    let Some(records) = run_view("hidepid") else {
        return;
    };
    assert_ancestor_and_current(&records);
    assert_eq!(records[1]["filtered"], true);
    // 同 UID 的 fixture 不证明哪一个外部 UID 被隐藏；仅证明真实非零过滤配置。
}

#[test]
fn proc_stat_overmount_has_a_different_actual_descriptor_mount_identity() {
    let Some(records) = run_view("overmount") else {
        return;
    };
    assert_ancestor_and_current(&records);
    let overmount = &records[2];
    assert_eq!(overmount["view"], "overmount");
    assert_ne!(overmount["root_mount_id"], overmount["record_mount_id"]);
}

fn assert_ancestor_and_current(records: &[Value]) {
    assert!(records.len() >= 2);
    assert_eq!(records[0]["view"], "ancestor");
    assert!(records[0]["nspid_count"].as_u64().unwrap() > 1);
    assert_ne!(records[0]["nspid_first"], records[0]["pid"]);
    assert_eq!(records[0]["nspid_last"], records[0]["pid"]);
    assert_eq!(records[1]["view"], "current");
    assert_eq!(records[1]["nspid_count"], 1);
    assert_eq!(records[1]["nspid_first"], records[1]["pid"]);
    for record in &records[..2] {
        assert_eq!(record["root_mount_id"], record["status_mount_id"]);
        assert_eq!(record["mount_record_found"], true);
    }
    assert_ne!(records[0]["root_mount_id"], records[1]["root_mount_id"]);
}

fn run_view(mode: &str) -> Option<Vec<Value>> {
    let mut process = ProbeChild::spawn(mode);
    let output = process.output();
    let text = std::str::from_utf8(&output.stdout).unwrap();
    let records: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    println!(
        "DG_LINUX_GROUP_VIEW mode={mode} exit={:?} records={records:?}",
        output.status.code()
    );
    if output.status.code() == Some(77) {
        assert!(matches!(mode, "namespace" | "hidepid" | "overmount"));
        let record = records.last().expect("explicit unavailable evidence");
        assert_eq!(record["qualification"], "unavailable");
        assert!(matches!(
            record["errno"].as_i64().unwrap(),
            1 | 13 | 22 | 38
        ));
        println!("DG_LINUX_GROUP_VIEW_SKIP mode={mode} QUALIFICATION_NOT_PROVEN");
        return None;
    }
    assert!(
        output.status.success(),
        "native probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!records.is_empty());
    for record in &records {
        assert_eq!(record["qualification"], "observed");
    }
    Some(records)
}
