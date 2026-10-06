//! 真实未连接服务器形成 pending Connect；不以 pending Read 代替。
use super::cleanup_progress::CleanupProgress;
use super::overlapped_pipe::OverlappedPipe;
use super::pipe_security::PipeSecurity;
use std::time::{Duration, Instant};

fn pending_connect() -> (Option<OverlappedPipe>, Vec<u16>, PipeSecurity) {
    let name: Vec<u16> = format!(r"\\.\pipe\diskgraph-prepared-{}", uuid::Uuid::new_v4())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let security = PipeSecurity::for_current_user().unwrap();
    let mut owner = None;
    OverlappedPipe::prepare_into(&name, &security, &mut owner).unwrap();
    owner.as_mut().unwrap().start_connect().unwrap();
    owner.as_ref().unwrap().io_witness().unwrap();
    (owner, name, security)
}

fn cleanup(pipe: &mut OverlappedPipe) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if pipe.poll_cleanup(deadline).unwrap() == CleanupProgress::Complete {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "original pending Connect did not complete"
        );
        std::thread::yield_now();
    }
}

#[test]
fn expired_connect_cleanup_retains_original_addresses_without_eof() {
    let (mut owner, _, _) = pending_connect();
    let pipe = owner.as_mut().unwrap();
    let before = pipe.io_witness().unwrap();
    assert_eq!(
        pipe.poll_cleanup(Instant::now()).unwrap(),
        CleanupProgress::Pending
    );
    assert_eq!(pipe.io_witness().unwrap(), before);
    assert!(!pipe.eof());
    cleanup(pipe);
    assert!(!pipe.eof(), "connect cancellation is not read EOF");
}

#[test]
fn query_failure_retains_original_connect_for_actual_retry() {
    let (mut owner, _, _) = pending_connect();
    let pipe = owner.as_mut().unwrap();
    let before = pipe.io_witness().unwrap();
    OverlappedPipe::fail_next_query_for_test();
    assert!(pipe.read_next().is_err());
    assert_eq!(pipe.io_witness().unwrap(), before);
    assert!(!pipe.eof());
    cleanup(pipe);
    assert!(!pipe.eof());
}

#[test]
fn actual_connect_completion_allows_read_then_distinct_read_cancellation() {
    let (mut owner, name, security) = pending_connect();
    let writer = OverlappedPipe::open_writer(&name, &security).unwrap();
    let pipe = owner.as_mut().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    // 原 Connect 完成后，read_next 提交真正的 Read；写端仍活着，不能正常 EOF。
    loop {
        assert!(pipe.read_next().unwrap().is_none());
        assert!(!pipe.eof());
        assert!(Instant::now() < deadline);
        assert!(pipe.start_connect().is_err());
        if !pipe.connecting_for_test() {
            break;
        }
        std::thread::yield_now();
    }
    pipe.io_witness().unwrap();
    cleanup(pipe);
    drop(writer);
}

#[test]
fn external_owner_survives_panic_after_actual_connect_submission() {
    let mut owner = None;
    let security = PipeSecurity::for_current_user().unwrap();
    let name: Vec<u16> = format!(r"\\.\pipe\diskgraph-panic-connect-{}", uuid::Uuid::new_v4())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        OverlappedPipe::prepare_into(&name, &security, &mut owner).unwrap();
        owner.as_mut().unwrap().start_connect().unwrap();
        owner.as_ref().unwrap().io_witness().unwrap();
        panic!("post-submit fixture");
    }));
    assert!(result.is_err());
    let pipe = owner.as_mut().unwrap();
    pipe.io_witness().unwrap();
    cleanup(pipe);
    assert!(!pipe.eof());
}

#[test]
fn pending_connect_moves_threads_without_replacing_original_storage() {
    let (mut owner, _, _) = pending_connect();
    let mut pipe = owner.take().unwrap();
    let witness = pipe.io_witness().unwrap();
    let addresses = (witness.0 as usize, witness.1 as usize, witness.2 as usize);
    let pipe = std::thread::spawn(move || {
        let witness = pipe.io_witness().unwrap();
        assert_eq!(
            (witness.0 as usize, witness.1 as usize, witness.2 as usize),
            addresses
        );
        cleanup(&mut pipe);
        assert!(!pipe.eof());
        pipe
    })
    .join()
    .unwrap();
    assert!(!pipe.eof());
}
