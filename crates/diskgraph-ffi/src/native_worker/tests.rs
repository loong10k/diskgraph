//! 唯一线程 ID 必须独立于已经被另一等待者取走的 JoinHandle。
use super::NativeWorker;
use std::sync::{Arc, mpsc::channel};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn self_wait_is_denied_even_after_another_waiter_takes_the_handle() {
    let (worker_tx, worker_rx) = channel::<Arc<NativeWorker>>();
    let (check_tx, check_rx) = channel();
    let (answer_tx, answer_rx) = channel();
    let worker = Arc::new(NativeWorker::new(thread::spawn(move || {
        let own_worker = worker_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        check_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let _ = answer_tx.send(own_worker.try_join());
    })));
    worker_tx.send(worker.clone()).unwrap();
    let joining = worker.clone();
    let joiner = thread::spawn(move || joining.join());
    let deadline = Instant::now() + Duration::from_secs(5);
    let taken = loop {
        if worker.thread.lock().unwrap().0.is_none() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(1));
    };
    let release = check_tx.send(());
    let answer = answer_rx.recv_timeout(Duration::from_secs(10));
    let joined = joiner.join();
    assert!(taken, "另一等待者尚未真正取走唯一 JoinHandle");
    assert!(release.is_ok());
    assert_eq!(
        answer.unwrap(),
        Err("coordinator cannot join its own thread")
    );
    joined.unwrap().unwrap();
    assert!(worker.is_joined());
}
