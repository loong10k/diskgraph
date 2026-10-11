//! 公开失败在执行栈结束前实际交付，不以恢复完成代替输出。
use super::deliver;
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;
use std::io::{self, Write};

/// 实际记录字节及flush边界的输出端。
#[derive(Default)]
struct ObservedOutput {
    bytes: Vec<u8>,
    flushed: bool,
    broken: bool,
}
impl Write for ObservedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.broken {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushed = true;
        Ok(())
    }
}

#[test]
fn original_failure_is_written_and_flushed_before_execution_returns() {
    let mut stdout = ObservedOutput::default();
    let mut stderr = Vec::new();
    let mut attempted = false;
    let result = deliver::<()>(
        Err(BusinessError::PermissionDenied.into()),
        Vec::new(),
        true,
        &mut stdout,
        &mut stderr,
        &mut attempted,
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    let response: serde_json::Value = serde_json::from_slice(&stdout.bytes).unwrap();
    assert_eq!(response["ok"], false);
    assert!(
        stdout.flushed,
        "private foreground completion may follow only after public flush"
    );
    assert!(
        attempted,
        "outer entry must not replay failure after recovery"
    );
    assert!(stderr.is_empty());
}

#[test]
fn broken_public_output_preserves_original_denial_and_disables_replay() {
    let mut stdout = ObservedOutput {
        broken: true,
        ..ObservedOutput::default()
    };
    let mut attempted = false;
    let result = deliver::<()>(
        Err(BusinessError::PermissionDenied.into()),
        vec!["partial".into()],
        true,
        &mut stdout,
        &mut Vec::new(),
        &mut attempted,
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    assert!(attempted);
}

#[test]
fn successful_response_is_flushed_without_generating_an_error() {
    let mut stdout = ObservedOutput::default();
    let mut attempted = false;
    assert_eq!(
        deliver(
            Ok(7),
            vec!["{\"ok\":true}".into()],
            true,
            &mut stdout,
            &mut Vec::new(),
            &mut attempted
        )
        .unwrap(),
        7
    );
    assert!(stdout.flushed);
    assert!(!attempted);
}

#[test]
fn text_failure_uses_only_original_stderr_and_flushes() {
    let mut stdout = Vec::new();
    let mut stderr = ObservedOutput::default();
    let mut attempted = false;
    let result = deliver::<()>(
        Err(BusinessError::NotFound.into()),
        Vec::new(),
        false,
        &mut stdout,
        &mut stderr,
        &mut attempted,
    );
    assert!(result.is_err());
    assert!(stdout.is_empty());
    assert!(!stderr.bytes.is_empty());
    assert!(stderr.flushed);
    assert!(attempted);
}
