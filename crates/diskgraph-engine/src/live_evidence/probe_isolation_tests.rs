//! 每次探针独立取消域的真实子进程回归；子进程 fixture 保留在 probe_tests 原路径。

use super::ProbeLimits;
#[cfg(windows)]
use super::native_probe_test_budget::NativeProbeTestBudget as ProbeBudget;
#[cfg(not(windows))]
use super::probe_budget::ProbeBudget;
use super::probe_execution::run_probe;
use super::probe_failure::ProbeFailure;
use super::probe_tests::{assert_heartbeat_stopped, fixture};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[test]
fn cancelling_one_sample_does_not_terminate_another() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    let release = temp.path().join("release-b");
    let config = ProbeLimits {
        timeout: Duration::from_secs(8),
        ..ProbeLimits::default()
    };
    std::thread::scope(|scope| {
        let cancelled = scope.spawn(|| {
            let mut command = fixture("cancellable_descendant");
            command.env("DG_PROBE_MARKER", &a);
            let mut budget = ProbeBudget::new(&config).unwrap();
            run_probe(&mut command, &mut budget)
        });
        let other = scope.spawn(|| {
            let mut command = fixture("heartbeat_until_release");
            command
                .env("DG_PROBE_MARKER", &b)
                .env("DG_PROBE_RELEASE", &release);
            let limits = ProbeLimits {
                timeout: Duration::from_secs(8),
                ..ProbeLimits::default()
            };
            let mut budget = ProbeBudget::new(&limits).unwrap();
            run_probe(&mut command, &mut budget)
        });
        let start = Instant::now();
        while !(a.exists() && b.exists()) {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "missing handshake"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        config.cancel.store(true, Ordering::Release);
        let cancelled_result = cancelled.join();

        // 第一项清理已经返回时，另一独立 fixture 仍须继续产生完整心跳。
        let started = Instant::now();
        let first = loop {
            if let Some(value) = std::fs::read_to_string(&b)
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
            {
                break Some(value);
            }
            if started.elapsed() >= Duration::from_secs(2) {
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let started = Instant::now();
        let mut advanced = false;
        if let Some(first_value) = first {
            while started.elapsed() < Duration::from_secs(2) {
                if std::fs::read_to_string(&b)
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .is_some_and(|value| value > first_value)
                {
                    advanced = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        std::fs::write(&release, b"go").unwrap();
        let other_result = other.join().unwrap();
        assert!(matches!(cancelled_result, Ok(Err(ProbeFailure::Cancelled))));
        assert!(
            first.is_some(),
            "independent fixture had no complete heartbeat"
        );
        assert!(
            advanced,
            "independent fixture stopped after the other cleanup"
        );
        assert_eq!(other_result.unwrap().exit_code, Some(0));
    });
    assert_heartbeat_stopped(&a);
}
