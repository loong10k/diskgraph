use super::ProbeLimits;
use super::probe_budget::ProbeBudget;
use super::probe_execution::run_probe;
use super::probe_failure::ProbeFailure;
use super::probe_tests::fixture;
use std::time::Duration;

#[test]
fn exiting_leaders_are_reaped_even_when_darwin_reports_eperm() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("pid");
    // 重复真实快速退出/超额输出，覆盖 XNU SRUN/INEXIT→SZOMB 观察过渡。
    // PID 由隔离 child 握手记录；waitpid 只检查本宿主的这个子进程。
    for attempt in 0..128 {
        let mut command = fixture("stamp_pid");
        command.env("DG_PROBE_MARKER", &marker);
        let config = ProbeLimits {
            timeout: Duration::from_secs(2),
            max_output_bytes: 4096,
            ..ProbeLimits::default()
        };
        let mut budget = ProbeBudget::new(&config).unwrap();
        let result = run_probe(&mut command, &mut budget);
        let pid: i32 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
        let mut status = 0;
        let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        let error = std::io::Error::last_os_error().raw_os_error();
        if reaped == 0 {
            // 失败时仍回收隔离 fixture；它自身有三秒 watchdog，避免测试泄漏。
            unsafe { libc::waitpid(pid, &mut status, 0) };
        }
        assert!(
            matches!(result, Err(ProbeFailure::OutputLimit)),
            "{attempt}: {result:?}"
        );
        assert_eq!(reaped, -1, "{attempt}: cleanup left an unreaped leader");
        assert_eq!(
            error,
            Some(libc::ECHILD),
            "{attempt}: unexpected ownership state"
        );
    }
}
