//! Windows 测试的单次探针写入耗时汇总，不编入生产运行时。
use super::git_private_write_profile::GitPrivateWriteProfile;
use std::io::Write;
use std::thread::ThreadId;
use std::time::Instant;

/// 原探针预算生命周期内的私有写入诊断；来源：原生 Rust Windows 性能定位。
/// 仅在显式开关开启时观察，不创建目录、不续期、不授予执行或恢复能力。
pub(super) struct GitPrivateWriteSummary {
    baseline: ([u128; 9], u64),
    started: Instant,
    thread: ThreadId,
}

impl GitPrivateWriteSummary {
    /// 参数：无；返回：显式诊断开启时的当前线程计数基线，否则无额外计时。
    pub(super) fn new() -> Option<Self> {
        (std::env::var_os("DG_PRIVATE_WRITE_PROFILE").as_deref() == Some(std::ffi::OsStr::new("1")))
            .then(|| Self {
                baseline: GitPrivateWriteProfile::snapshot(),
                started: Instant::now(),
                thread: std::thread::current().id(),
            })
    }
}

impl Drop for GitPrivateWriteSummary {
    fn drop(&mut self) {
        // 线程本地累计量不能跨线程相减；转移后的预算只报告不可归因，绝不伪造阶段耗时。
        let mut output = std::io::stdout().lock();
        if self.thread != std::thread::current().id() {
            let _ = writeln!(
                output,
                "DG_PRIVATE_WRITE_SUMMARY attribution=thread_changed"
            );
            return;
        }
        let (stages, writes) = GitPrivateWriteProfile::snapshot();
        let elapsed: [u128; 9] =
            std::array::from_fn(|index| stages[index].saturating_sub(self.baseline.0[index]));
        let _ = writeln!(
            output,
            "DG_PRIVATE_WRITE_SUMMARY writes={} elapsed_us={} stage_us={elapsed:?} stages=preflight,parent_lease,parent_identity,open_create,write,finish_write,observe,finish_operation,return thread={:?}",
            writes.saturating_sub(self.baseline.1),
            self.started.elapsed().as_micros(),
            self.thread,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Output};

    fn run_small_session(enabled: bool) -> Output {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "live_evidence::git_private_capacity_tests::owned_overwrites_replace_the_existing_allocation_charge",
            "--nocapture",
        ]);
        command.env_remove("DG_PRIVATE_WRITE_PROFILE");
        if enabled {
            command.env("DG_PRIVATE_WRITE_PROFILE", "1");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "native write fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"),
            "must execute the actual private write fixture"
        );
        output
    }

    #[test]
    fn small_completed_session_reports_its_actual_eight_writes() {
        let output = run_small_session(true);
        let text = String::from_utf8(output.stdout).unwrap();
        let summaries: Vec<_> = text
            .lines()
            .filter(|line| line.starts_with("DG_PRIVATE_WRITE_SUMMARY writes=8 "))
            .collect();
        assert_eq!(summaries.len(), 1, "{text}");
        assert!(summaries[0].contains("stage_us=["));
        assert!(summaries[0].contains("parent_lease,parent_identity,open_create,write"));
    }

    #[test]
    fn disabled_session_does_not_emit_or_change_native_write_result() {
        let output = run_small_session(false);
        assert!(
            !String::from_utf8(output.stdout)
                .unwrap()
                .contains("DG_PRIVATE_WRITE_SUMMARY")
        );
    }
}
