//! 执行输入独立于扫描响应预算；来源：PF-06 序列化准入，不模拟原生管道成功。
use crate::scan_worker_input::ScanWorkerInput;
use diskgraph_scan_worker::{ProtocolLimits, WorkerRequest};

fn request(limits: ProtocolLimits) -> WorkerRequest {
    let root = tempfile::tempdir().unwrap();
    WorkerRequest::scan(root.path(), &Default::default(), limits).unwrap()
}

#[test]
fn small_response_frame_does_not_reject_an_admissible_request() {
    let limits = ProtocolLimits {
        max_frame_bytes: 128,
        max_stream_bytes: 1024,
        max_nodes: 10,
        max_depth: 2,
    };
    let request = request(limits);
    assert!(serde_json::to_vec(&request).unwrap().len() > 128);
    assert!(ScanWorkerInput::new(&request).is_ok());
}

#[test]
fn large_response_budget_does_not_admit_a_request_above_helper_input_limit() {
    let limits = ProtocolLimits {
        max_frame_bytes: 16 * 1024 * 1024,
        max_stream_bytes: 32 * 1024 * 1024,
        max_nodes: 10,
        max_depth: 2,
    };
    #[cfg(unix)]
    let root = std::path::PathBuf::from(format!("/{}", "a".repeat(400_000)));
    #[cfg(windows)]
    let root = std::path::PathBuf::from(format!("C:\\{}", "a".repeat(400_000)));
    let request = WorkerRequest::scan(&root, &Default::default(), limits).unwrap();
    assert!(ScanWorkerInput::new(&request).is_err());
}
