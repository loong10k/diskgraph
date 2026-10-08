//! 已完成命令与未完成清理的组合语义，不将此投影测试当作原生退休验收。
use super::git_output::execution_error;
use super::probe_failure::ProbeFailure;
use super::probe_output::ProbeOutput;

fn output(code: i32) -> ProbeOutput {
    ProbeOutput {
        exit_code: Some(code),
        stdout: b"partial result".to_vec(),
        stderr: b"corrupt reflog".to_vec(),
    }
}

#[test]
fn nonzero_exit_keeps_command_error_and_original_cleanup_failure() {
    let failure = output(128)
        .with_cleanup(Err(ProbeFailure::Unsupported("original owner retained")))
        .unwrap_err();
    let message = execution_error(failure);
    assert!(
        message.starts_with("git exit status Some(128);"),
        "{message}"
    );
    assert!(message.contains("corrupt reflog"), "{message}");
    assert!(
        message.contains("cleanup also failed: probe unsupported: original owner retained"),
        "{message}"
    );
}

#[test]
fn successful_exit_still_refuses_unconfirmed_cleanup() {
    let failure = output(0)
        .with_cleanup(Err(ProbeFailure::Unsupported("original owner retained")))
        .unwrap_err();
    assert!(matches!(
        failure,
        ProbeFailure::Unsupported("original owner retained")
    ));
}

#[test]
fn cleaned_nonzero_output_preserves_existing_git_fallback_contract() {
    let result = output(128).with_cleanup(Ok(())).unwrap();
    assert_eq!(result.exit_code, Some(128));
    assert_eq!(result.stderr, b"corrupt reflog");
}

#[test]
fn latched_error_clone_shares_charged_stderr_and_keeps_product_error_bounded() {
    let failure = output(128)
        .with_cleanup(Err(ProbeFailure::Unsupported("original owner retained")))
        .unwrap_err();
    let cloned = failure.clone();
    let ProbeFailure::Cleanup { primary, .. } = &failure else {
        panic!("missing primary");
    };
    let ProbeFailure::Cleanup {
        primary: copied, ..
    } = &cloned
    else {
        panic!("missing cloned primary");
    };
    let ProbeFailure::CommandExit { stderr, .. } = primary.as_ref() else {
        panic!("missing exit status");
    };
    let ProbeFailure::CommandExit { stderr: copied, .. } = copied.as_ref() else {
        panic!("missing cloned exit status");
    };
    assert!(std::sync::Arc::ptr_eq(stderr, copied));
    assert_eq!(
        super::git_product_error::GitProductError::from_failure(Some(&failure)),
        super::git_product_error::GitProductError::Unavailable
    );
}
