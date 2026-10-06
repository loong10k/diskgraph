//! 重试策略的可移植回归；仅验证错误/预算控制，不代替 Linux 原生验收。
use super::bounded_open_retry::bounded_open_retry;
use diskgraph_core::ProcessEvidenceFailureCode as Failure;
use std::cell::Cell;

#[test]
fn transient_native_race_recovers_without_resetting_check() {
    let calls = Cell::new(0);
    let checks = Cell::new(0);
    let result = bounded_open_retry(
        &|| {
            checks.set(checks.get() + 1);
            Ok(())
        },
        &mut || {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                Err(std::io::Error::from_raw_os_error(11))
            } else {
                Ok(42)
            }
        },
        11,
    )
    .unwrap();
    assert_eq!(result.unwrap(), 42);
    assert_eq!(calls.get(), 2);
    assert_eq!(checks.get(), 4);
}

#[test]
fn persistent_native_race_has_finite_attempts() {
    let calls = Cell::new(0);
    let result = bounded_open_retry::<()>(
        &|| Ok(()),
        &mut || {
            calls.set(calls.get() + 1);
            Err(std::io::Error::from_raw_os_error(11))
        },
        11,
    )
    .unwrap();
    assert_eq!(result.unwrap_err().raw_os_error(), Some(11));
    assert_eq!(calls.get(), 8);
}

#[test]
fn non_retryable_error_is_immediate() {
    let calls = Cell::new(0);
    let result = bounded_open_retry::<()>(
        &|| Ok(()),
        &mut || {
            calls.set(calls.get() + 1);
            Err(std::io::Error::from_raw_os_error(13))
        },
        11,
    )
    .unwrap();
    assert_eq!(result.unwrap_err().raw_os_error(), Some(13));
    assert_eq!(calls.get(), 1);
}

#[test]
fn original_denial_stops_before_retry() {
    for denial in [
        Failure::PermissionDenied,
        Failure::Cancelled,
        Failure::BudgetExceeded,
    ] {
        let checks = Cell::new(0);
        let calls = Cell::new(0);
        let result = bounded_open_retry::<()>(
            &|| {
                checks.set(checks.get() + 1);
                if checks.get() == 3 {
                    Err(denial)
                } else {
                    Ok(())
                }
            },
            &mut || {
                calls.set(calls.get() + 1);
                Err(std::io::Error::from_raw_os_error(11))
            },
            11,
        );
        assert_eq!(result.unwrap_err(), denial);
        assert_eq!(calls.get(), 1);
    }
}

#[test]
fn successful_value_is_dropped_when_original_check_expires() {
    struct Held<'a>(&'a Cell<bool>);
    impl Drop for Held<'_> {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let dropped = Cell::new(false);
    let checks = Cell::new(0);
    let result = bounded_open_retry(
        &|| {
            checks.set(checks.get() + 1);
            if checks.get() == 2 {
                Err(Failure::BudgetExceeded)
            } else {
                Ok(())
            }
        },
        &mut || Ok(Held(&dropped)),
        11,
    );
    assert!(matches!(result, Err(Failure::BudgetExceeded)));
    assert!(dropped.get());
}
