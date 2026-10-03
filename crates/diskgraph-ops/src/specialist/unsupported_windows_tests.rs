use crate::OpsError;
use std::path::PathBuf;

use super::*;

#[test]
fn bounded_external_tool_execution_refuses_without_a_windows_sandbox() {
    let spec = CommandSpec {
        program: PathBuf::from("missing-tool.exe"),
        args: Vec::new(),
        cwd: None,
        env: Vec::new(),
        timeout_ms: 1000,
        max_output_bytes: 1024,
        retries: 0,
    };
    assert!(matches!(
        SandboxedRunner.run(&spec),
        Err(OpsError::Stale(message)) if message.contains("unsupported")
    ));
}
