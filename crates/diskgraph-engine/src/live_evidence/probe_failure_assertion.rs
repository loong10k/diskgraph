//! 测试断言保留原失败与清理诊断的边界；不改变生产错误传播。
use super::probe_failure::ProbeFailure;

/// 参数：真实探针结果及预期原失败判定；返回：断言原失败，否则中止测试。
pub(super) fn assert_probe_failure<T>(
    result: Result<T, ProbeFailure>,
    expected: impl FnOnce(&ProbeFailure) -> bool,
) {
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("probe unexpectedly succeeded"),
    };
    let mut primary = &error;
    // 仅沿 primary 链选择业务原因，不能将 cleanup 的 Deadline 当作原 Deadline。
    while let ProbeFailure::Cleanup {
        primary: original, ..
    } = primary
    {
        primary = original;
    }
    assert!(
        expected(primary),
        "original probe failure with retained cleanup: {error:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::ProbeFailure;
    use super::assert_probe_failure;

    #[test]
    fn nested_cleanup_preserves_the_expected_primary() {
        assert_probe_failure::<()>(
            Err(ProbeFailure::Cancelled
                .with_cleanup(Err(ProbeFailure::Deadline))
                .with_cleanup(Err(ProbeFailure::Unsupported("pending original owner")))),
            |error| matches!(error, ProbeFailure::Cancelled),
        );
    }

    #[test]
    fn cleanup_reason_cannot_substitute_for_original_failure() {
        let rejected = std::panic::catch_unwind(|| {
            assert_probe_failure::<()>(
                Err(ProbeFailure::OutputLimit.with_cleanup(Err(ProbeFailure::Deadline))),
                |error| matches!(error, ProbeFailure::Deadline),
            );
        });
        assert!(rejected.is_err());
        let success = std::panic::catch_unwind(|| {
            assert_probe_failure(Ok(()), |_| true);
        });
        assert!(success.is_err());
    }
}
