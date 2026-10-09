//! D44 真实外连 writer 的原时钟阶段诊断；来源：控制写守卫，不复制其重试算法。
use crate::StoreError;
use crate::control_write_deadline::ControlWriteDeadline;
use crate::process_job_enqueue_fixture::ProcessEnqueueFixture;
use rusqlite::Connection;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

// 两案验证不同因果边界：731ms 受400ms原请求限制；40ms须先于仍有效请求返回原BUSY。
// 后者2s只保证宿主比较阶段资格，非响应门限；两案总耗时仍严格小于550ms。
fn writer_phase_probe(host_busy_ms: u64, request_limited: bool) {
    let fixture = ProcessEnqueueFixture::new();
    let blocking_locks: bool = fixture
        .store
        .connection
        .query_row(
            "SELECT sqlite_compileoption_used('ENABLE_SETLK_TIMEOUT')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    fixture.set_busy_timeout(host_busy_ms);
    let before = fixture.persisted_state();
    let path = fixture.database_path();
    let held = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&held);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let locker = std::thread::spawn(move || {
        let connection = Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        held.store(true, Ordering::SeqCst);
        ready_tx.send(()).unwrap();
        // 旧实现也能有限退出；主线程先保存阶段结果再释放、join，避免断言遗留外部 owner。
        let _ = release_rx.recv_timeout(Duration::from_millis(1200));
        held.store(false, Ordering::SeqCst);
        connection.execute_batch("ROLLBACK").unwrap();
    });
    if let Err(error) = ready_rx.recv_timeout(Duration::from_secs(5)) {
        let _ = release_tx.send(());
        let joined = locker.join();
        panic!("external writer failed to hold its transaction: {error:?}; {joined:?}");
    }

    let call_wall_started = Instant::now();
    // 仅400ms请求案注入250ms已消耗年龄，非真实准备墙钟；不靠sleep过调度取得过期结果。
    // host40保留原2000ms资格窗口与实际250ms准备等待，两案不作同设置因果比较。
    let started = if request_limited {
        let Some(original_started) = call_wall_started.checked_sub(Duration::from_millis(250))
        else {
            let _ = release_tx.send(());
            locker.join().unwrap();
            panic!("fixture cannot represent the injected original request age");
        };
        original_started
    } else {
        call_wall_started
    };
    let original_window_ms = if request_limited { 400 } else { 2000 };
    let deadline = started + Duration::from_millis(original_window_ms);
    if !request_limited {
        std::thread::sleep(Duration::from_millis(250));
    }
    let preconsume_elapsed = started.elapsed();
    let remaining_at_call = deadline.saturating_duration_since(Instant::now());
    let held_at_call = observed.load(Ordering::SeqCst);
    let new_started = Instant::now();
    let created = ControlWriteDeadline::new(
        &fixture.store.connection,
        deadline,
        fixture.authority.expires_at_unix_seconds(),
    );
    let new_elapsed = new_started.elapsed();
    let new_succeeded = created.is_ok();
    let (result, remaining_before_begin, begin_elapsed, finish_elapsed, autocommit_before_finish) =
        match created {
            Ok(window) => {
                let remaining_before_begin = deadline.saturating_duration_since(Instant::now());
                let begin_started = Instant::now();
                let result = window.begin();
                let begin_elapsed = begin_started.elapsed();
                let autocommit_before_finish = fixture.store.connection.is_autocommit();
                let finish_started = Instant::now();
                let result = window.finish(result);
                (
                    result,
                    Some(remaining_before_begin),
                    Some(begin_elapsed),
                    Some(finish_started.elapsed()),
                    autocommit_before_finish,
                )
            }
            Err(error) => (
                Err(error),
                None,
                None,
                None,
                fixture.store.connection.is_autocommit(),
            ),
        };
    let total_elapsed = call_wall_started.elapsed();
    let budget_age_elapsed = started.elapsed();
    let remaining_at_return = deadline.saturating_duration_since(Instant::now());
    let returned_while_held = observed.load(Ordering::SeqCst);
    let autocommit_at_return = fixture.store.connection.is_autocommit();
    let _ = release_tx.send(());
    locker.join().unwrap();

    // 恢复与重开先于耗时断言；诊断只在普通 Rust 栈输出，不进入任何 SQLite FFI 回调。
    let after = fixture.persisted_state();
    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        std::thread::sleep(remaining + Duration::from_millis(20));
    }
    fixture.assert_connection_restored(host_busy_ms as i64);
    eprintln!(
        "D44_WRITER_PHASE {}",
        serde_json::json!({
            "sqlite_version": rusqlite::version(),
            "sqlite_setlk_timeout_enabled": blocking_locks,
            "host_busy_ms": host_busy_ms,
            "original_window_ms": original_window_ms,
            "preconsume_injected": request_limited,
            "injected_preconsume_ms": if request_limited { 250 } else { 0 },
            "preconsume_is_measured_wall": !request_limited,
            "preconsume_us": preconsume_elapsed.as_micros(),
            "remaining_at_call_us": remaining_at_call.as_micros(),
            "new_us": new_elapsed.as_micros(),
            "new_succeeded": new_succeeded,
            "remaining_before_begin_us": remaining_before_begin.map(|value| value.as_micros()),
            "begin_us": begin_elapsed.map(|value| value.as_micros()),
            "finish_us": finish_elapsed.map(|value| value.as_micros()),
            "total_us": total_elapsed.as_micros(),
            "call_wall_us": total_elapsed.as_micros(),
            "budget_age_us": budget_age_elapsed.as_micros(),
            "remaining_at_return_us": remaining_at_return.as_micros(),
            "held_at_call": held_at_call,
            "returned_while_held": returned_while_held,
            "autocommit_before_finish": autocommit_before_finish,
            "autocommit_at_return": autocommit_at_return,
            "all_tables_unchanged": after == before,
            "result": format!("{result:?}")
        })
    );
    assert!(
        remaining_at_call > Duration::ZERO,
        "initial preparation already expired"
    );
    assert!(
        held_at_call && new_succeeded,
        "writer/guard stage was not reached"
    );
    assert!(
        returned_while_held,
        "guard waited until external owner release"
    );
    assert!(autocommit_before_finish && autocommit_at_return);
    assert_eq!(after, before, "persistent control state changed");
    if request_limited {
        assert!(
            matches!(result, Err(StoreError::BudgetExceeded)),
            "actual={result:?}"
        );
        assert_eq!(remaining_at_return, Duration::ZERO);
        assert!(
            budget_age_elapsed < Duration::from_millis(550),
            "original deadline refreshed: budget_age={budget_age_elapsed:?}, call_wall={total_elapsed:?}"
        );
    } else {
        assert!(
            remaining_before_begin.is_some_and(|value| value > Duration::from_millis(40)),
            "host40 comparison did not reach BEGIN with a full host timeout remaining"
        );
        assert!(
            matches!(&result, Err(StoreError::Sqlite(rusqlite::Error::SqliteFailure(code, _))) if code.extended_code == rusqlite::ffi::SQLITE_BUSY),
            "actual={result:?}"
        );
        assert!(
            remaining_at_return > Duration::ZERO,
            "host's shorter timeout was expanded"
        );
    }
    assert!(
        total_elapsed < Duration::from_millis(550),
        "strict original wall bound failed: {total_elapsed:?}"
    );
}

#[test]
fn control_write_external_writer_original_deadline_has_real_stage_diagnostics() {
    writer_phase_probe(731, true);
}

#[test]
fn control_write_external_writer_short_host_busy_is_not_request_expiry() {
    writer_phase_probe(40, false);
}
