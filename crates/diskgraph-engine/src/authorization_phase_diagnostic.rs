//! 调试构建的授权预算失败定位；不输出资源、主体、token 或原错误内容。
use crate::EngineError;
use diskgraph_core::BusinessError;
use std::io::Write;
use std::time::{Duration, Instant};

/// 在显式启用时记录原预算失败的阶段。参数：phase 为内部固定标签，operation 为原操作；返回：原结果。
/// 来源：OpenSpec Q-02 原生失败定位；不重试、不刷新期限，release 构建不启用。
pub(super) fn observe<T>(
    phase: &'static str,
    operation: impl FnOnce() -> Result<T, EngineError>,
) -> Result<T, EngineError> {
    let enabled = cfg!(debug_assertions)
        && std::env::var_os("DISKGRAPH_QUERY_DIAGNOSTICS").is_some_and(|value| value == "1");
    observe_with(enabled, phase, operation, |phase, elapsed, cpu_elapsed| {
        let _ = emit_budget_diagnostic(&mut std::io::stderr().lock(), phase, elapsed, cpu_elapsed);
    })
}

fn observe_with<T>(
    enabled: bool,
    phase: &'static str,
    operation: impl FnOnce() -> Result<T, EngineError>,
    emit: impl FnOnce(&'static str, Duration, Option<Duration>),
) -> Result<T, EngineError> {
    if !enabled {
        return operation();
    }
    let started = Instant::now();
    let cpu = crate::thread_cpu_observation::ThreadCpuObservation::start(true);
    let result = operation();
    let budget_failure = result.as_ref().is_err_and(|error| match error.primary() {
        EngineError::Business(BusinessError::BudgetExceeded) => true,
        EngineError::Store(error) => {
            matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                || error.is_busy()
                || error.is_interrupted()
        }
        _ => false,
    });
    if budget_failure {
        // CPU采样包含在墙钟区间内，日志写入位于两种观察之外。
        let cpu_elapsed = cpu.elapsed();
        emit(phase, started.elapsed(), cpu_elapsed);
    }
    result
}

fn emit_budget_diagnostic(
    writer: &mut impl Write,
    phase: &'static str,
    elapsed: Duration,
    cpu: Option<Duration>,
) -> std::io::Result<()> {
    writeln!(
        writer,
        "diskgraph: authorization_budget_phase={phase} elapsed_us={}",
        elapsed.as_micros()
    )?;
    if let Some(cpu) = cpu {
        writeln!(
            writer,
            "diskgraph: authorization_cpu_phase={phase} elapsed_us={} thread_cpu_us={}",
            elapsed.as_micros(),
            cpu.as_micros()
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{emit_budget_diagnostic, observe_with};
    use crate::EngineError;
    use diskgraph_core::BusinessError;

    #[test]
    fn cpu_diagnostic_preserves_original_line_and_numeric_cpu() {
        let mut output = Vec::new();
        emit_budget_diagnostic(
            &mut output,
            "terminal_ownership_sql",
            std::time::Duration::from_micros(100),
            Some(std::time::Duration::from_micros(5)),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "diskgraph: authorization_budget_phase=terminal_ownership_sql elapsed_us=100\ndiskgraph: authorization_cpu_phase=terminal_ownership_sql elapsed_us=100 thread_cpu_us=5\n"
        );
    }

    #[test]
    fn unknown_cpu_does_not_emit_a_zero_cpu_claim() {
        let mut output = Vec::new();
        emit_budget_diagnostic(
            &mut output,
            "terminal_ownership_sql",
            std::time::Duration::from_micros(100),
            None,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "diskgraph: authorization_budget_phase=terminal_ownership_sql elapsed_us=100\n"
        );
    }

    #[test]
    fn diagnostic_write_failure_does_not_replace_the_original_budget_error() {
        let result: Result<(), _> = observe_with(
            true,
            "terminal_ownership_sql",
            || Err(EngineError::Business(BusinessError::BudgetExceeded)),
            |phase, elapsed, _| {
                let mut exhausted: &mut [u8] = &mut [];
                assert!(emit_budget_diagnostic(&mut exhausted, phase, elapsed, None).is_err());
            },
        );
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ));
    }

    #[test]
    fn disabled_observation_preserves_the_original_failure_without_emission() {
        let result: Result<(), _> = observe_with(
            false,
            "terminal_reader_open",
            || Err(EngineError::Business(BusinessError::BudgetExceeded)),
            |_, _, _| panic!("disabled diagnostics emitted"),
        );
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ));
    }

    #[test]
    fn enabled_budget_observation_emits_once_and_keeps_original_cleanup_payload() {
        let mut labels = Vec::new();
        let result: Result<(), _> = observe_with(
            true,
            "terminal_ownership_sql",
            || {
                Err(EngineError::WithCleanup {
                    primary: Box::new(EngineError::Business(BusinessError::BudgetExceeded)),
                    cleanup: std::io::Error::other("private path must never be logged"),
                })
            },
            |label, _, _| labels.push(label),
        );
        assert_eq!(labels, ["terminal_ownership_sql"]);
        let Err(EngineError::WithCleanup { cleanup, .. }) = result else {
            panic!("original cleanup payload was replaced")
        };
        assert_eq!(cleanup.to_string(), "private path must never be logged");
    }

    #[test]
    fn success_and_permission_denial_do_not_emit_budget_diagnostics() {
        assert_eq!(
            observe_with(
                true,
                "terminal_control",
                || Ok(7),
                |_, _, _| panic!("success emitted")
            )
            .unwrap(),
            7
        );
        let denied: Result<(), _> = observe_with(
            true,
            "terminal_control",
            || Err(EngineError::Business(BusinessError::PermissionDenied)),
            |_, _, _| panic!("denial emitted"),
        );
        assert!(matches!(
            denied,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ));
    }
}
