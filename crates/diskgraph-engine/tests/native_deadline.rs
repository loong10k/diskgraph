#![cfg(any(target_os = "linux", target_os = "macos", windows))]
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;
use diskgraph_engine::native_deadline::ClockStamp;
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};
#[test]
fn delayed_adoption_never_extends_the_original_local_deadline() {
    let original = Instant::now() + Duration::from_secs(2);
    let stamp = ClockStamp::capture(original).unwrap();
    std::thread::sleep(Duration::from_millis(40));
    let adopted = stamp.adopt(Duration::from_secs(2)).unwrap();
    assert!(
        adopted <= original,
        "queued stamp must not renew consumed time"
    );
}
#[test]
fn expired_original_is_not_captured() {
    assert!(matches!(
        ClockStamp::capture(Instant::now() - Duration::from_secs(1)),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}
#[test]
fn actual_other_process_cannot_renew_expired_original_budget() {
    let stamp = ClockStamp::capture(Instant::now() + Duration::from_millis(80)).unwrap();
    assert_eq!(child(&stamp, "expired").code(), Some(51));
}
#[test]
fn actual_other_process_adopts_live_original_budget() {
    let stamp = ClockStamp::capture(Instant::now() + Duration::from_secs(30)).unwrap();
    assert!(child(&stamp, "live").success());
}
#[test]
fn wrong_domain_version_and_excessive_future_expiry_are_refused() {
    let stamp = ClockStamp::capture(Instant::now() + Duration::from_secs(2)).unwrap();
    for (field, value) in [
        ("domain", serde_json::json!([99, 0, 0])),
        ("version", serde_json::json!(99)),
        ("expires_nanos", serde_json::json!(u64::MAX)),
    ] {
        let mut wire = serde_json::to_value(&stamp).unwrap();
        wire[field] = value;
        let changed: ClockStamp = serde_json::from_value(wire).unwrap();
        assert!(
            changed.adopt(Duration::from_secs(30)).is_err(),
            "invalid {field} must not be adopted"
        );
    }
    assert!(stamp.adopt(Duration::ZERO).is_err());
    let mut wire = serde_json::to_value(&stamp).unwrap();
    wire["unknown"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ClockStamp>(wire).is_err());
}
fn child(stamp: &ClockStamp, mode: &str) -> ExitStatus {
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "stamp_child", "--nocapture"])
        .env(
            "DG_CLOCK_STAMP_FIXTURE",
            serde_json::to_string(stamp).unwrap(),
        )
        .env("DG_CLOCK_MODE", mode)
        .spawn()
        .unwrap();
    let mut guard = OriginalChild(Some(child));
    let limit = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = guard.0.as_mut().unwrap().try_wait().unwrap() {
            guard.0.take();
            return status;
        }
        assert!(
            Instant::now() < limit,
            "original child did not finish under original observation deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
struct OriginalChild(Option<Child>);
impl Drop for OriginalChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
#[test]
#[ignore = "actual parent test invokes this child entry explicitly"]
fn stamp_child() {
    let wire = std::env::var("DG_CLOCK_STAMP_FIXTURE").unwrap();
    assert!(wire.len() < 256);
    let stamp: ClockStamp = serde_json::from_str(&wire).unwrap();
    let mode = std::env::var("DG_CLOCK_MODE").unwrap();
    if mode == "expired" {
        std::thread::sleep(Duration::from_millis(150));
    }
    #[cfg(target_os = "linux")]
    if mode == "namespace" {
        use std::os::unix::fs::MetadataExt;
        let actual = std::fs::File::open("/proc/self/ns/time")
            .unwrap()
            .metadata()
            .unwrap();
        let original = serde_json::to_value(&stamp).unwrap();
        let actual_domain = serde_json::json!([1, actual.dev(), actual.ino()]);
        assert_ne!(
            original["domain"], actual_domain,
            "actual child must enter a different kernel namespace"
        );
        eprintln!(
            "DG_CLOCK_ACTUAL_NAMESPACE_CHANGED=1 original={} actual={actual_domain}",
            original["domain"]
        );
    }
    match (mode.as_str(), stamp.adopt(Duration::from_secs(30))) {
        ("namespace", Err(EngineError::Business(BusinessError::Conflict))) => {
            eprintln!("DG_CLOCK_ACTUAL_NAMESPACE_REFUSED=1");
            std::process::exit(54)
        }
        ("live", Ok(_)) => {}
        ("expired", Err(EngineError::Business(BusinessError::BudgetExceeded))) => {
            std::process::exit(51)
        }
        ("expired", Ok(_)) => std::process::exit(53),
        _ => std::process::exit(52),
    }
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires explicit disposable Linux CI namespace privilege"]
fn actual_linux_time_namespace_mismatch_is_refused() {
    assert_eq!(std::env::var("GITHUB_ACTIONS").unwrap(), "true");
    assert_eq!(
        std::env::var("RUNNER_ENVIRONMENT").unwrap(),
        "github-hosted"
    );
    assert_eq!(std::env::var("RUNNER_OS").unwrap(), "Linux");
    let stamp = ClockStamp::capture(Instant::now() + Duration::from_secs(30)).unwrap();
    let executable = std::env::current_exe().unwrap();
    // 仅临时 CI 子 namespace；timeout 管理自身原进程组，不修改宿主时钟或安装材料。
    let child = std::process::Command::new("/usr/bin/sudo")
        .args([
            "-n",
            "/usr/bin/timeout",
            "--kill-after=1s",
            "5s",
            "/usr/bin/unshare",
            "--fork",
            "--kill-child",
            "--time",
            "--monotonic=600",
            "--",
            "/usr/bin/env",
        ])
        .arg(format!(
            "DG_CLOCK_STAMP_FIXTURE={}",
            serde_json::to_string(&stamp).unwrap()
        ))
        .arg("DG_CLOCK_MODE=namespace")
        .arg(executable)
        .args(["--ignored", "--exact", "stamp_child", "--nocapture"])
        .spawn()
        .unwrap();
    let mut guard = OriginalChild(Some(child));
    let until = Instant::now() + Duration::from_secs(8);
    let status = loop {
        if let Some(status) = guard.0.as_mut().unwrap().try_wait().unwrap() {
            guard.0.take();
            break status;
        }
        assert!(
            Instant::now() < until,
            "original namespace wrapper did not complete"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        status.code(),
        Some(54),
        "actual namespace mismatch must reject as Conflict, not silently consume unrelated clock values"
    );
}
