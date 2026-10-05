//! PF-06 真实 Read 错误与 cancel 发布竞争；原错误必须优先于其派生停止标志。
use super::{WorkerControl, gated_reader::GatedReader, hooks};
use crate::{FrameReader, ProtocolLimits};
use diskgraph_disktree_core::scan::ScanProgress;
use serde_json::json;
use std::{
    io,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

fn assert_real_error_keeps_priority(kind: io::ErrorKind, expected_kind: &str) {
    let (release, gate) = mpsc::channel();
    let (ready, observed_ready) = mpsc::channel();
    let (returned, observed_read) = mpsc::channel();
    let (published, observed_publish) = mpsc::channel();
    let (witness, observed_hook) = mpsc::channel();
    hooks::install_reader_witnesses(returned, published);
    let reader = FrameReader::new(
        GatedReader { gate, ready, kind },
        ProtocolLimits {
            max_frame_bytes: 1024,
            max_stream_bytes: 4096,
            max_nodes: 1,
            max_depth: 1,
        },
    );
    let control =
        Arc::new(WorkerControl::start(reader, Arc::new(ScanProgress::default())).unwrap());
    let checking = Arc::clone(&control);
    let check_thread = thread::spawn(move || {
        hooks::install_after_empty(move |lock_available| {
            // old: mutex 已释放，必须等实际错误+cancel发布后才继续；不是靠sleep撞竞态。
            // fixed: check自己保留guard，reader已真实返回后允许check先完成并释放guard。
            witness.send(lock_available).unwrap();
            observed_ready
                .recv_timeout(Duration::from_secs(5))
                .expect("actual Read entered before its gate is released");
            release.send(()).unwrap();
            observed_read
                .recv_timeout(Duration::from_secs(5))
                .expect("actual Read returned its requested error");
            if lock_available {
                observed_publish
                    .recv_timeout(Duration::from_secs(5))
                    .expect(
                        "actual reader error and cancellation published before old check continues",
                    );
            }
        });
        checking.check()
    });
    let first = check_thread.join();
    // 即使第一检查失败或hook超时，也先真实join控制reader，之后才断言，不遗弃测试线程。
    let mut control = Arc::try_unwrap(control)
        .ok()
        .expect("check thread released its Arc");
    let joined = control.finish();
    let second = control.check();
    let observed_hook = observed_hook.recv_timeout(Duration::from_secs(5));
    let first = first.expect("qualified check thread must finish");
    let first_json = first
        .as_ref()
        .err()
        .map(|error| serde_json::to_value(error).unwrap());
    let second_json = second
        .as_ref()
        .err()
        .map(|error| serde_json::to_value(error).unwrap());
    eprintln!(
        "CONTROL_ERROR_RACE {}",
        json!({"requested_kind":expected_kind,"hook_lock_available":observed_hook.as_ref().ok(),"first_error":first_json,"after_join_error":second_json,"joined":joined.is_ok()})
    );
    assert!(
        observed_hook.is_ok(),
        "empty-slot hook was actually reached"
    );
    joined.expect("real control reader must join");
    let error = if first.is_ok() {
        second_json.expect("real published protocol error must remain after actual join")
    } else {
        first_json.expect("an already delivered original protocol error is also valid")
    };
    assert_eq!(error["code"], "protocol");
    assert_eq!(error["io_kind"], expected_kind);
    assert_eq!(error["message"], "qualified control Read failure");
}

#[test]
fn unexpected_eof_published_during_check_is_never_reclassified_as_cancelled() {
    assert_real_error_keeps_priority(io::ErrorKind::UnexpectedEof, "unexpected_eof");
}

#[test]
fn invalid_data_published_during_check_is_never_reclassified_as_cancelled() {
    assert_real_error_keeps_priority(io::ErrorKind::InvalidData, "invalid_data");
}
