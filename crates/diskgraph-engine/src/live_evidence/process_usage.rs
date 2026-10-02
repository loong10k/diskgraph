//! 按精确路径采样可见占用并保留覆盖诊断。

use super::sampling_clock::now_ms;
use super::{ProcessHolder, UsageCoverage, UsageSample};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 按精确路径采样可见占用并保留覆盖诊断。
/// 参数：lsof 为受信程序路径，paths 为精确采样路径。
/// 返回：使用样本；无法运行/观察时不可当作无人占用。
/// Samples open handles for exact paths with `lsof`. The program is fixed,
/// the arguments are structured and there is no shell. Child polling uses a
/// 15-second timeout, but pipe draining has no hard byte/deadline bound in
/// this compatibility implementation. A missing or failing lsof is an
/// `Unobservable` sample, never evidence that a file is unused.
pub fn sample_process_usage(lsof: &Path, paths: &[&Path]) -> UsageSample {
    let sampled_at_unix_ms = now_ms();
    if paths.is_empty() {
        return UsageSample {
            sampled_at_unix_ms,
            coverage: UsageCoverage::Full,
            holders: Vec::new(),
        };
    }
    let mut command = Command::new(lsof);
    // -w: suppress warnings; -F pcn0: machine-readable pid/command/name
    // records, NUL-terminated; `--`: end of options, so a path that begins
    // with `-` is still a path.
    command.args(["-w", "-F", "pcn0", "--"]);
    command.args(paths);
    command.env_clear();
    if let Some(path_env) = std::env::var_os("PATH") {
        command.env("PATH", path_env);
    }
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.stdin(Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return UsageSample {
                sampled_at_unix_ms,
                coverage: UsageCoverage::Unobservable {
                    reason: format!("the handle probe could not run: {error}"),
                },
                holders: Vec::new(),
            };
        }
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                return UsageSample {
                    sampled_at_unix_ms,
                    coverage: UsageCoverage::Unobservable {
                        reason: format!("the handle probe failed: {error}"),
                    },
                    holders: Vec::new(),
                };
            }
        }
    }
    let mut output = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut output);
    }
    let exit = child
        .try_wait()
        .ok()
        .flatten()
        .and_then(|status| status.code());
    if timed_out {
        return UsageSample {
            sampled_at_unix_ms,
            coverage: UsageCoverage::Unobservable {
                reason: "the handle probe timed out".into(),
            },
            holders: Vec::new(),
        };
    }
    if exit != Some(0) {
        // lsof exits 1 when it found nothing at all — which is a real
        // answer, not a failure; any other code is a probe problem.
        if exit == Some(1) && output.is_empty() {
            return UsageSample {
                sampled_at_unix_ms,
                coverage: UsageCoverage::Full,
                holders: Vec::new(),
            };
        }
        return UsageSample {
            sampled_at_unix_ms,
            coverage: UsageCoverage::Unobservable {
                reason: format!("the handle probe exited with {exit:?}"),
            },
            holders: Vec::new(),
        };
    }
    // Parse `p<pid>\0c<comm>\0n<path>\0` records into holders.
    let text = String::from_utf8_lossy(&output);
    let mut holders: Vec<ProcessHolder> = Vec::new();
    let mut current_pid: Option<u32> = None;
    let mut current_command: Option<String> = None;
    // lsof prints the kernel's view of a path (on macOS `/var` is
    // `/private/var`) and marks deleted-but-open files with a suffix, so
    // matching canonicalizes what was asked for and strips the marker.
    let canonical_asked: Vec<PathBuf> = paths
        .iter()
        .map(|asked| asked.canonicalize().unwrap_or_else(|_| asked.to_path_buf()))
        .collect();
    for record in text.split('\0') {
        let record = record.trim_start_matches('\n');
        if record.is_empty() {
            continue;
        }
        let (tag, value) = record.split_at(1);
        match tag {
            "p" => {
                current_pid = value.parse().ok();
                current_command = None;
            }
            "c" => current_command = Some(value.to_owned()),
            "n" => {
                let reported = value.strip_suffix(" (deleted)").unwrap_or(value);
                // Only paths that were asked for count as holders.
                if let (Some(pid), Some(command_name)) = (current_pid, current_command.clone())
                    && canonical_asked
                        .iter()
                        .any(|asked| asked.to_string_lossy() == reported)
                {
                    holders.push(ProcessHolder {
                        pid,
                        command: command_name,
                    });
                }
            }
            _ => {}
        }
    }
    holders.sort();
    holders.dedup();
    UsageSample {
        sampled_at_unix_ms,
        coverage: UsageCoverage::Full,
        holders,
    }
}
