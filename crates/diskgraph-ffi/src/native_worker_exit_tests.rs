//! PF-06 协调线程退出回归：真实 TLS 析构阻塞不能被逻辑 finished 状态替代。
use crate::native_worker_exit_barrier::serialize_tls_fixture;
use crate::spawn_job;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

thread_local! {
    static EXIT_GATE: RefCell<Option<WorkerExitGate>> = const { RefCell::new(None) };
}

/// 真实 worker 的线程局部析构屏障；来源：Rust std::thread TLS 生命周期验收。
struct WorkerExitGate {
    reached: Sender<()>,
    release: Receiver<()>,
    released: Sender<bool>,
}

impl Drop for WorkerExitGate {
    fn drop(&mut self) {
        let _ = self.reached.send(());
        // 有限救援期限只属于测试；不修改产品作业/扫描期限。
        let released = self.release.recv_timeout(Duration::from_secs(60)).is_ok();
        let _ = self.released.send(released);
    }
}

#[test]
fn result_json_waits_for_worker_tls_exit_while_poll_remains_nonblocking() {
    let _fixture = serialize_tls_fixture();
    let (reached_tx, reached_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (released_tx, released_rx) = channel();
    let (exit_tx, exit_rx) = channel();
    let handle = spawn_job(move |_, _| {
        EXIT_GATE.with(|gate| {
            *gate.borrow_mut() = Some(WorkerExitGate {
                reached: reached_tx,
                release: release_rx,
                released: released_tx,
            });
        });
        // 先在普通 work 阶段等待所有调用者 ready，不能在 TLS 中等待新线程启动。
        exit_rx
            .recv_timeout(Duration::from_secs(60))
            .map_err(|error| error.to_string())?;
        Ok(json!({"marker":"real_worker_tls_exit"}))
    });

    let (poll_tx, poll_rx) = channel();
    let poll_handle = handle.clone();
    let (poll_ready_tx, poll_ready_rx) = channel();
    let (poll_start_tx, poll_start_rx) = channel();
    let poller = thread::spawn(move || {
        let _ = poll_ready_tx.send(());
        poll_start_rx.recv_timeout(Duration::from_secs(60)).unwrap();
        let _ = poll_tx.send(poll_handle.poll_result_json());
    });

    let (calling_tx, calling_rx) = channel();
    let (result_tx, result_rx) = channel();
    let result_handle = handle.clone();
    let (reader_ready_tx, reader_ready_rx) = channel();
    let (reader_start_tx, reader_start_rx) = channel();
    let reader = thread::spawn(move || {
        let _ = reader_ready_tx.send(());
        reader_start_rx
            .recv_timeout(Duration::from_secs(60))
            .unwrap();
        let _ = calling_tx.send(());
        let result = result_handle.result_json();
        let _ = result_tx.send(result.clone());
        result
    });
    let poll_ready = poll_ready_rx.recv_timeout(Duration::from_secs(5));
    let reader_ready = reader_ready_rx.recv_timeout(Duration::from_secs(5));
    let exit_sent = exit_tx.send(());
    // 析构开始意味着 work 与状态写入已返回；既有调用线程此时才开始调用。
    let reached = reached_rx.recv_timeout(Duration::from_secs(10));
    let logically_finished = handle.is_finished();
    let poll_started = poll_start_tx.send(());
    let reader_started = reader_start_tx.send(());
    let poll_before_release = poll_rx.recv_timeout(Duration::from_secs(5));
    let calling = calling_rx.recv_timeout(Duration::from_secs(5));
    let early_result = result_rx.recv_timeout(Duration::from_millis(250));

    // 先救援释放并回收所有测试拥有的线程，再断言；RED 不留下被屏障锁住的线程。
    let release_sent = release_tx.send(());
    let released = released_rx.recv_timeout(Duration::from_secs(10));
    let poll_join = poller.join();
    let reader_join = reader.join();

    assert!(
        reached.is_ok(),
        "worker 未进入真实 TLS 析构阶段: {reached:?}"
    );
    assert!(
        poll_ready.is_ok() && reader_ready.is_ok(),
        "调用者必须先启动: {poll_ready:?}/{reader_ready:?}"
    );
    assert!(exit_sent.is_ok() && poll_started.is_ok() && reader_started.is_ok());
    assert!(logically_finished, "TLS 屏障必须位于 finished 写入之后");
    assert!(calling.is_ok(), "result 调用线程未到达入口: {calling:?}");
    assert!(release_sent.is_ok(), "TLS 屏障未等到救援释放");
    assert!(released.unwrap(), "不能把析构超时当作主动释放");
    poll_join.unwrap();
    let result: Value = serde_json::from_str(&reader_join.unwrap()).unwrap();
    let polled: Value = serde_json::from_str(&poll_before_release.unwrap().unwrap()).unwrap();
    assert_eq!(polled["ok"], true);
    assert_eq!(polled["data"]["marker"], "real_worker_tls_exit");
    assert_eq!(result, polled, "join 不得改变结果 JSON 或重新执行工作");
    assert!(
        matches!(
            early_result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "worker TLS 仍被屏障阻塞时 result_json 已返回: {early_result:?}"
    );
    // 旧 spawn_job 丢弃 JoinHandle，故 RED 仅证明过早返回；不宣称已 join 原 worker。
    // GREEN 必须由产品唯一 owner 的真实 join 满足上述退出顺序，不能轮询 finished 充数。
}

#[test]
fn concurrent_result_waiters_share_the_actual_worker_join() {
    let _fixture = serialize_tls_fixture();
    let (reached_tx, reached_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (released_tx, released_rx) = channel();
    let (exit_tx, exit_rx) = channel();
    let handle = spawn_job(move |_, _| {
        EXIT_GATE.with(|gate| {
            *gate.borrow_mut() = Some(WorkerExitGate {
                reached: reached_tx,
                release: release_rx,
                released: released_tx,
            });
        });
        // 先在普通 work 阶段等待所有调用者 ready，不能在 TLS 中等待新线程启动。
        exit_rx
            .recv_timeout(Duration::from_secs(60))
            .map_err(|error| error.to_string())?;
        Ok(json!({"marker":"shared_worker_join"}))
    });
    let (calling_tx, calling_rx) = channel();
    let (ready_tx, ready_rx) = channel();
    let mut starts = Vec::new();
    let (result_tx, result_rx) = channel();
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let handle = handle.clone();
            let calling = calling_tx.clone();
            let result = result_tx.clone();
            let ready = ready_tx.clone();
            let (start_tx, start_rx) = channel();
            starts.push(start_tx);
            thread::spawn(move || {
                let _ = ready.send(());
                start_rx.recv_timeout(Duration::from_secs(60)).unwrap();
                let _ = calling.send(());
                let answer = handle.result_json();
                let _ = result.send(answer.clone());
                answer
            })
        })
        .collect();
    let first_ready = ready_rx.recv_timeout(Duration::from_secs(5));
    let second_ready = ready_rx.recv_timeout(Duration::from_secs(5));
    let exit_sent = exit_tx.send(());
    let reached = reached_rx.recv_timeout(Duration::from_secs(10));
    let started: Vec<_> = starts.into_iter().map(|start| start.send(())).collect();
    let first_call = calling_rx.recv_timeout(Duration::from_secs(5));
    let second_call = calling_rx.recv_timeout(Duration::from_secs(5));
    let early_result = result_rx.recv_timeout(Duration::from_millis(250));
    let release_sent = release_tx.send(());
    let released = released_rx.recv_timeout(Duration::from_secs(10));
    let answers: Vec<_> = readers.into_iter().map(thread::JoinHandle::join).collect();

    assert!(reached.is_ok(), "真实 TLS 阶段未到达: {reached:?}");
    assert!(
        first_ready.is_ok() && second_ready.is_ok(),
        "调用者必须先启动: {first_ready:?}/{second_ready:?}"
    );
    assert!(exit_sent.is_ok() && started.iter().all(Result::is_ok));
    assert!(first_call.is_ok() && second_call.is_ok());
    assert!(release_sent.is_ok());
    assert!(released.unwrap());
    let answers: Vec<_> = answers.into_iter().map(Result::unwrap).collect();
    assert_eq!(answers[0], answers[1]);
    let answer: Value = serde_json::from_str(&answers[0]).unwrap();
    assert_eq!(answer["ok"], true);
    assert_eq!(answer["data"]["marker"], "shared_worker_join");
    assert!(
        matches!(
            early_result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "另一个等待者取走 JoinHandle 不等于线程已退出: {early_result:?}"
    );
}

#[test]
fn worker_panic_is_joined_and_keeps_the_existing_error_envelope() {
    let handle = spawn_job(|_, _| panic!("真实工作闭包恐慌，验证原错误和线程回收"));
    let answer = handle.result_json();
    let decoded: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(decoded["ok"], false);
    assert_eq!(decoded["error"], "scan worker crashed");
    assert_eq!(handle.result_json(), answer);
    assert_eq!(handle.poll_result_json(), Some(answer));
}
