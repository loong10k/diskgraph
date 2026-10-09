//! Windows探针显式出生/展开恢复边界；原child在catch外，失败仍占原宿主容量。
use super::collect_output;
use crate::live_evidence::probe_budget::ProbeBudget;
use crate::live_evidence::probe_failure::ProbeFailure;
use crate::live_evidence::probe_output::ProbeOutput;
use crate::live_evidence::probe_windows::WindowsProbeChild;
use crate::native_child::{ChildError, ChildInputMode, CleanupProgress, WindowsChild};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::process::Command;
use std::time::Duration;

/// 参数：command为受信命令，budget持有原期限、输出、取消与宿主；返回：完整输出或原失败。
/// 出生前预留容量；一次处置失败移交原owner后才投影错误，panic原payload原样继续。
pub(super) fn execute(
    command: &mut Command,
    budget: &mut ProbeBudget,
) -> Result<ProbeOutput, ProbeFailure> {
    budget.check()?;
    let registry = budget
        .probe_registry
        .clone()
        .ok_or(ProbeFailure::Unsupported(
            "Windows probe requires an external recovery host",
        ))?;
    let reservation = registry.reserve().map_err(|error| match error {
        crate::EngineError::Business(diskgraph_core::BusinessError::ResourceExhausted) => {
            ProbeFailure::ResourceLimit
        }
        other => ProbeFailure::Io(other.to_string()),
    })?;
    // 两个状态槽都在catch外；从出生owner转到输出适配之间没有外部调用或分配。
    let mut born = None;
    let mut running = None;
    #[cfg(test)]
    let profile = (std::env::var_os("DG_PRIVATE_WRITE_PROFILE").as_deref()
        == Some(std::ffi::OsStr::new("1")))
    .then(std::time::Instant::now);
    #[cfg(test)]
    let spawn_elapsed = std::cell::Cell::new(None);
    let observed = catch_unwind(AssertUnwindSafe(|| {
        {
            // 两阶段检查依次借同一原预算；不复制账本，不以调用次数推断出生。
            let original_budget = std::cell::RefCell::new(&mut *budget);
            WindowsChild::spawn_into_with_admission(
                command,
                ChildInputMode::Null,
                &mut born,
                || original_budget.borrow_mut().check(),
                || original_budget.borrow_mut().check(),
            )
            .map_err(ProbeFailure::from)?;
        }
        #[cfg(test)]
        if let Some(started) = profile {
            spawn_elapsed.set(Some(started.elapsed()));
        }
        running = born.take().map(WindowsProbeChild::from_child);
        let child = running
            .as_mut()
            .ok_or(ProbeFailure::Unsupported("probe owner absent"))?;
        collect_output(child, budget)
    }));
    #[cfg(test)]
    let observed_elapsed = profile.map(|started| started.elapsed());
    let owner = born
        .take()
        .or_else(|| running.take().map(WindowsProbeChild::into_child));
    // 保持原生错误直到同一个owner已回收或放回预留槽，不能依靠Drop再尝试来释放容量。
    let cleanup: Result<(), ChildError> = match owner {
        Some(mut child) => {
            let mut progress = child.poll_cleanup(budget.deadline());
            // Pending不是失败；仅成功输出可在同一原预算内继续观察同一个owner。
            // 取消、到期、真实清理错误和panic不续等，仍由外部宿主接管原容量。
            while matches!(progress, Ok(CleanupProgress::Pending))
                && matches!(&observed, Ok(Ok(_)))
                && budget.check().is_ok()
            {
                std::thread::sleep(
                    Duration::from_millis(1).min(
                        budget
                            .deadline()
                            .saturating_duration_since(std::time::Instant::now()),
                    ),
                );
                if budget.check().is_err() {
                    break;
                }
                progress = child.poll_cleanup(budget.deadline());
            }
            match progress {
                Ok(CleanupProgress::Complete) => Ok(()),
                Ok(CleanupProgress::Pending) => {
                    // 不调用兼容cleanup/Drop续等；同一原槽保留Job及全部pending I/O。
                    reservation.retain(child);
                    Err(ChildError::Unsupported(
                        "probe cleanup pending; original owner retained",
                    ))
                }
                Err(error) => {
                    reservation.retain(child);
                    Err(error)
                }
            }
        }
        None => Ok(()),
    };
    #[cfg(test)]
    if let (Some(started), Some(observed_elapsed)) = (profile, observed_elapsed) {
        // 只报告同一次原生执行的阶段，不记录命令参数、路径、输出或凭据。
        // 同线程编号与写入汇总关联；写日志失败不替换原结果或 panic payload。
        use std::io::Write;
        let spawn_us = spawn_elapsed.get().map(|elapsed| elapsed.as_micros());
        let collect_us = spawn_elapsed
            .get()
            .map(|elapsed| observed_elapsed.saturating_sub(elapsed).as_micros());
        let cleanup_us = started
            .elapsed()
            .saturating_sub(observed_elapsed)
            .as_micros();
        let _ = writeln!(
            std::io::stdout().lock(),
            "DG_PROBE_EXECUTION thread={:?} spawn_us={spawn_us:?} collect_us={collect_us:?} cleanup_us={cleanup_us} observed_ok={} cleanup_ok={}",
            std::thread::current().id(),
            matches!(&observed, Ok(Ok(_))),
            cleanup.is_ok(),
        );
    }
    match observed {
        Ok(Ok(output)) => {
            // 成功输出也先复核原请求；到期/取消保持主原因，清理Pending仅作次诊断。
            output.finish(budget, cleanup.map_err(ProbeFailure::from))
        }
        Ok(Err(primary)) => Err(primary.with_cleanup(cleanup.map_err(ProbeFailure::from))),
        Err(payload) => resume_unwind(payload),
    }
}
