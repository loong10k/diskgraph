use super::ProbeLimits;
use super::probe_budget::ProbeBudget;
use super::probe_execution::run_probe;
use super::probe_failure::ProbeFailure;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

pub(super) fn fixture(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "live_evidence::probe_tests::probe_child_fixture",
            "--nocapture",
            "--quiet",
        ])
        .env_clear()
        .env("DG_PROBE_CASE", mode);
    command
}

fn limits(bytes: usize, timeout: Duration) -> ProbeLimits {
    ProbeLimits {
        max_output_bytes: bytes,
        timeout,
        ..ProbeLimits::default()
    }
}

#[test]
fn probe_child_fixture() {
    let Ok(mode) = std::env::var("DG_PROBE_CASE") else {
        return;
    };
    // 独立有限寿命 watchdog：实际 RED 不能永久挂住 cargo 或留下后代。
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(3));
        std::process::exit(88);
    });
    match mode.as_str() {
        "both" | "flood" => {
            let count = if mode == "both" { 4096 } else { 192 << 10 };
            std::thread::scope(|scope| {
                scope.spawn(|| std::io::stderr().write_all(&vec![b'e'; count]).unwrap());
                std::io::stdout().write_all(&vec![b'o'; count]).unwrap();
            });
        }
        "tiny" => {
            std::io::stdout().write_all(b"marker").unwrap();
        }
        "stamp_pid" => {
            std::fs::write(
                std::env::var_os("DG_PROBE_MARKER").unwrap(),
                std::process::id().to_string(),
            )
            .unwrap();
            std::io::stdout().write_all(&[b'o'; 8192]).unwrap();
        }
        "stamp" => {
            std::fs::write(std::env::var_os("DG_PROBE_MARKER").unwrap(), b"started").unwrap();
        }
        "environment" => {
            assert_eq!(std::env::var("DG_PROBE_EXPECTED").unwrap(), "kept");
            assert!(std::env::var_os("HOME").is_none());
            assert!(std::env::var_os("GIT_CONFIG_COUNT").is_none());
            std::io::stdout()
                .write_all(b"controlled-environment")
                .unwrap();
        }
        "silent" => {
            std::thread::sleep(Duration::from_secs(2));
        }
        "abnormal" => {
            #[cfg(unix)]
            unsafe {
                libc::raise(libc::SIGTERM);
            }
            #[cfg(windows)]
            std::process::exit(-1);
        }
        #[cfg(unix)]
        "escaped_leader" => {
            assert_eq!(
                unsafe { libc::setpgid(0, libc::getpgid(libc::getppid())) },
                0
            );
            std::thread::sleep(Duration::from_secs(2));
        }
        "short" => {
            std::thread::sleep(Duration::from_millis(150));
        }
        "descendant" | "closed_descendant" => {
            let mut child = fixture("heartbeat");
            child.env(
                "DG_PROBE_MARKER",
                std::env::var_os("DG_PROBE_MARKER").unwrap(),
            );
            if mode == "closed_descendant" {
                child.stdout(Stdio::null()).stderr(Stdio::null());
            }
            // 本隔离 fixture 必须退出而不等待后代，以复现 leader 已退出的边界。
            // 外层 runner 终止自有组，后代另有 watchdog；仅此测试允许不 wait。
            #[allow(clippy::zombie_processes)]
            let _child = child.spawn().unwrap();
            // 等待实际握手再退出，确保后代已活动而非仅被请求启动。
            let marker = std::env::var_os("DG_PROBE_MARKER").unwrap();
            while !std::path::Path::new(&marker).exists() {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        "heartbeat" | "heartbeat_then_exit" => {
            let marker = std::env::var_os("DG_PROBE_MARKER").unwrap();
            for index in 0..if mode == "heartbeat" { 500 } else { 60 } {
                std::fs::write(&marker, index.to_string()).unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        "stream" => loop {
            std::io::stdout().write_all(&[b'o'; 4096]).unwrap();
            std::io::stderr().write_all(&[b'e'; 4096]).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        },
        #[cfg(unix)]
        "auto_reap" => {
            // 仅在隔离子宿主修改 SIGCHLD；主测试进程和并发样本不受影响。
            unsafe {
                libc::signal(libc::SIGCHLD, libc::SIG_IGN);
            }
            let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
            assert!(matches!(
                run_probe(&mut fixture("tiny"), &mut budget),
                Err(ProbeFailure::Unsupported(_))
            ));
            std::io::stdout().write_all(b"auto-reap-refused").unwrap();
        }
        "argv" => {
            for argument in std::env::args_os()
                .skip_while(|value| value != "--")
                .skip(1)
            {
                let text = argument.to_string_lossy();
                std::io::stdout().write_all(text.as_bytes()).unwrap();
                std::io::stdout().write_all(b"\0").unwrap();
            }
        }
        other => panic!("unknown fixture {other}"),
    }
    std::process::exit(0);
}

#[test]
fn both_streams_are_drained_while_the_child_runs() {
    let mut budget = ProbeBudget::new(&limits(1 << 20, Duration::from_secs(2))).unwrap();
    let output = run_probe(&mut fixture("flood"), &mut budget).unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.stderr.len(), 192 << 10);
    assert!(output.stdout.ends_with(&vec![b'o'; 192 << 10]));
}

#[test]
fn stdout_and_stderr_share_one_byte_limit() {
    let mut budget = ProbeBudget::new(&limits(6000, Duration::from_secs(2))).unwrap();
    assert!(matches!(
        run_probe(&mut fixture("both"), &mut budget),
        Err(ProbeFailure::OutputLimit)
    ));
}

#[test]
fn output_is_not_retained_beyond_the_limit() {
    let mut budget = ProbeBudget::new(&limits(2000, Duration::from_secs(2))).unwrap();
    assert!(matches!(
        run_probe(&mut fixture("both"), &mut budget),
        Err(ProbeFailure::OutputLimit)
    ));
}

#[test]
fn exact_and_zero_output_limits_do_not_fake_eof() {
    let mut baseline = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let bytes = run_probe(&mut fixture("tiny"), &mut baseline)
        .unwrap()
        .stdout
        .len();
    let mut exact = ProbeBudget::new(&limits(bytes, Duration::from_secs(2))).unwrap();
    assert!(run_probe(&mut fixture("tiny"), &mut exact).is_ok());
    let mut short = ProbeBudget::new(&limits(bytes - 1, Duration::from_secs(2))).unwrap();
    assert!(matches!(
        run_probe(&mut fixture("tiny"), &mut short),
        Err(ProbeFailure::OutputLimit)
    ));
    let mut zero = ProbeBudget::new(&limits(0, Duration::from_secs(2))).unwrap();
    assert!(matches!(
        run_probe(&mut fixture("tiny"), &mut zero),
        Err(ProbeFailure::OutputLimit)
    ));
}

#[test]
fn command_outputs_accumulate_in_the_sample_budget() {
    let mut generous = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let bytes = run_probe(&mut fixture("tiny"), &mut generous)
        .unwrap()
        .stdout
        .len();
    let mut budget = ProbeBudget::new(&limits(bytes * 2 - 1, Duration::from_secs(2))).unwrap();
    run_probe(&mut fixture("tiny"), &mut budget).unwrap();
    assert!(matches!(
        run_probe(&mut fixture("tiny"), &mut budget),
        Err(ProbeFailure::OutputLimit)
    ));
    assert!(matches!(
        run_probe(&mut fixture("tiny"), &mut budget),
        Err(ProbeFailure::OutputLimit)
    ));
}

#[test]
fn deadline_applies_to_a_silent_child() {
    let mut budget = ProbeBudget::new(&limits(1 << 20, Duration::from_millis(100))).unwrap();
    let start = Instant::now();
    assert!(matches!(
        run_probe(&mut fixture("silent"), &mut budget),
        Err(ProbeFailure::Deadline)
    ));
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn multiple_commands_do_not_reset_the_deadline() {
    let mut budget = ProbeBudget::new(&limits(1 << 20, Duration::from_millis(270))).unwrap();
    run_probe(&mut fixture("short"), &mut budget).unwrap();
    assert!(matches!(
        run_probe(&mut fixture("short"), &mut budget),
        Err(ProbeFailure::Deadline)
    ));
}

#[test]
fn cancellation_before_spawn_and_during_wait_is_not_success() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("started");
    let config = ProbeLimits::default();
    config.cancel.store(true, Ordering::Release);
    let mut budget = ProbeBudget::new(&config).unwrap();
    let mut stamp = fixture("stamp");
    stamp.env("DG_PROBE_MARKER", &marker);
    assert!(matches!(
        run_probe(&mut stamp, &mut budget),
        Err(ProbeFailure::Cancelled)
    ));
    assert!(!marker.exists(), "pre-cancelled probe executed");
    config.cancel.store(false, Ordering::Release);
    let mut budget = ProbeBudget::new(&config).unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            config.cancel.store(true, Ordering::Release);
        });
        assert!(matches!(
            run_probe(&mut fixture("silent"), &mut budget),
            Err(ProbeFailure::Cancelled)
        ));
    });
}

fn assert_heartbeat_stopped(path: &std::path::Path) {
    std::thread::sleep(Duration::from_millis(80));
    let before = std::fs::read(path).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        std::fs::read(path).unwrap(),
        before,
        "descendant continued after cleanup"
    );
}

#[test]
fn an_exited_leader_does_not_make_inherited_pipes_complete() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("beat");
    let mut command = fixture("descendant");
    command.env("DG_PROBE_MARKER", &marker);
    let mut budget = ProbeBudget::new(&limits(1 << 20, Duration::from_millis(400))).unwrap();
    assert!(matches!(
        run_probe(&mut command, &mut budget),
        Err(ProbeFailure::Deadline)
    ));
    assert_heartbeat_stopped(&marker);
}

#[test]
fn normal_success_also_cleans_ordinary_descendants() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("beat");
    let mut command = fixture("closed_descendant");
    command.env("DG_PROBE_MARKER", &marker);
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    assert!(run_probe(&mut command, &mut budget).is_ok());
    assert_heartbeat_stopped(&marker);
}

#[test]
fn spawn_failure_is_sticky_and_stops_later_commands() {
    let mut command = Command::new("/nonexistent/dg-probe-child");
    command.env_clear();
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    assert!(run_probe(&mut command, &mut budget).is_err());
    assert!(run_probe(&mut fixture("tiny"), &mut budget).is_err());
}

#[test]
fn continuous_output_does_not_starve_deadline_checks() {
    let mut budget = ProbeBudget::new(&limits(64 << 20, Duration::from_millis(250))).unwrap();
    assert!(matches!(
        run_probe(&mut fixture("stream"), &mut budget),
        Err(ProbeFailure::Deadline)
    ));
}

#[test]
fn cancelling_one_sample_does_not_terminate_another() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    let config = ProbeLimits::default();
    std::thread::scope(|scope| {
        let cancelled = scope.spawn(|| {
            let mut command = fixture("descendant");
            command.env("DG_PROBE_MARKER", &a);
            let mut budget = ProbeBudget::new(&config).unwrap();
            run_probe(&mut command, &mut budget)
        });
        scope.spawn(|| {
            let start = Instant::now();
            while !(a.exists() && b.exists()) {
                assert!(
                    start.elapsed() < Duration::from_secs(2),
                    "missing handshake"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            config.cancel.store(true, Ordering::Release);
        });
        let mut other = fixture("heartbeat_then_exit");
        other.env("DG_PROBE_MARKER", &b);
        let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
        assert_eq!(
            run_probe(&mut other, &mut budget).unwrap().exit_code,
            Some(0)
        );
        assert!(matches!(
            cancelled.join().unwrap(),
            Err(ProbeFailure::Cancelled)
        ));
    });
    assert_heartbeat_stopped(&a);
}

#[cfg(unix)]
#[test]
fn a_host_that_auto_reaps_is_refused_without_changing_its_signal_policy() {
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let output = run_probe(&mut fixture("auto_reap"), &mut budget).unwrap();
    assert!(output.stdout.ends_with(b"auto-reap-refused"));
}

#[test]
fn native_argv_and_explicit_environment_preserve_their_contract() {
    let arguments = [
        "",
        "a b",
        "中文",
        "quoted\"value",
        "trailing\\",
        "a\\\"b",
        "$(literal);*?",
        "--dash",
    ];
    let mut command = fixture("argv");
    command.arg("--").args(arguments);
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let output = run_probe(&mut command, &mut budget).unwrap();
    let expected = arguments.join("\0") + "\0";
    assert!(output.stdout.ends_with(expected.as_bytes()), "{output:?}");
    let mut command = fixture("environment");
    command.env("DG_PROBE_EXPECTED", "kept");
    let output = run_probe(&mut command, &mut budget).unwrap();
    assert!(output.stdout.ends_with(b"controlled-environment"));
}

#[test]
fn an_argument_with_nul_is_rejected_before_a_child_executes() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("started");
    let mut command = fixture("stamp");
    command.arg("bad\0argument").env("DG_PROBE_MARKER", &marker);
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    assert!(run_probe(&mut command, &mut budget).is_err());
    assert!(!marker.exists());
}

#[test]
fn abnormal_termination_is_a_failed_observation() {
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    assert!(
        run_probe(&mut fixture("abnormal"), &mut budget).is_err(),
        "abnormal termination became a completed observation"
    );
    assert!(
        run_probe(&mut fixture("tiny"), &mut budget).is_err(),
        "a failed sample started another command"
    );
}

#[cfg(unix)]
#[test]
fn a_failed_cleanup_is_reported_with_the_primary_failure() {
    let mut command = fixture("silent");
    command.env("DG_PROBE_CLEANUP_FAULT", "1");
    let mut budget = ProbeBudget::new(&limits(1 << 20, Duration::from_millis(100))).unwrap();
    let error = run_probe(&mut command, &mut budget).unwrap_err();
    let text = error.to_string();
    assert!(
        text.contains("deadline") && text.contains("cleanup also failed"),
        "{text}"
    );
    assert!(matches!(budget.check(), Err(ProbeFailure::Deadline)));
}

#[cfg(unix)]
#[test]
fn an_escaped_leader_is_terminated_without_signalling_its_new_group() {
    let mut budget = ProbeBudget::new(&limits(1 << 20, Duration::from_millis(100))).unwrap();
    let started = Instant::now();
    let error = run_probe(&mut fixture("escaped_leader"), &mut budget).unwrap_err();
    assert!(error.to_string().contains("deadline"), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "waited for escaped leader instead of terminating it"
    );
}
