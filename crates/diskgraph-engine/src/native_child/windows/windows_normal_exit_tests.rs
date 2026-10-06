//! 正常完成许可的真实 Windows 资格；不把 End、EOF 或 leader 单独退出当成 Job 空。

use super::super::{ChildError, ChildSpawnError};
use super::windows_normal_exit_test_support::{WindowsNormalExitTestSupport, check};
use std::cell::Cell;
use std::io;

#[test]
fn normal_exit_waits_for_stdio_closed_descendant_after_leader_has_exited() {
    let mut fixture = WindowsNormalExitTestSupport::spawn("leader-with-descendant").unwrap();
    let result = (|| -> Result<_, ChildError> {
        fixture.await_eof()?;
        let code = fixture.await_leader_exit()?;
        fixture.close_control()?;
        fixture.bind_descendant()?;
        let heartbeat = fixture.heartbeat_advances("descendant-heartbeat")?;
        fixture.descendant_alive()?;
        let active = fixture.active()?;
        let exited = fixture.leader_exited()?;
        let deadline = fixture.deadline();
        let pending = fixture.child.poll_normal_exit(|| check(deadline));
        fixture.descendant_alive()?;
        fixture.release_and_observe()?;
        let empty = fixture.active()?;
        let complete = fixture.child.poll_normal_exit(|| check(deadline));
        Ok((code, heartbeat, active, exited, pending, empty, complete))
    })();
    let natural = fixture.release_and_observe();
    let marker = fixture.natural_marker("descendant-natural");
    natural.unwrap();
    let (code, heartbeat, active, exited, pending, empty, complete) = result.unwrap();
    assert_eq!(
        code, 0,
        "watchdog or child failure cannot qualify natural exit"
    );
    assert!(heartbeat.1 > heartbeat.0);
    assert!(
        active > 0,
        "the full original Job must remain nonempty while the bound ordinary descendant is alive"
    );
    assert!(exited, "held duplicate leader must already be signaled");
    assert!(
        matches!(pending, Ok(false)),
        "normal permission while descendant is alive: {pending:?}"
    );
    assert!(
        marker,
        "descendant must pass its independent natural release gate"
    );
    assert_eq!(
        empty, 0,
        "held duplicate Job must independently confirm no active process"
    );
    assert!(
        matches!(complete, Ok(true)),
        "no normal permission after natural exit: {complete:?}"
    );
    assert_eq!(fixture.child.exit_code(), Some(0));
}

#[test]
fn normal_exit_does_not_accept_end_and_two_eofs_while_leader_remains_alive() {
    let mut fixture = WindowsNormalExitTestSupport::spawn("live-leader").unwrap();
    let result = (|| -> Result<_, ChildError> {
        fixture.await_eof()?;
        fixture.close_control()?;
        let heartbeat = fixture.heartbeat_advances("leader-heartbeat")?;
        fixture.leader_alive()?;
        let active = fixture.active()?;
        let exited = fixture.leader_exited()?;
        let deadline = fixture.deadline();
        let pending = fixture.child.poll_normal_exit(|| check(deadline));
        fixture.leader_alive()?;
        fixture.release_and_observe()?;
        let complete = fixture.child.poll_normal_exit(|| check(deadline));
        Ok((heartbeat, active, exited, pending, complete))
    })();
    let natural = fixture.release_and_observe();
    let marker = fixture.natural_marker("leader-natural");
    natural.unwrap();
    let (heartbeat, active, exited, pending, complete) = result.unwrap();
    assert!(heartbeat.1 > heartbeat.0);
    assert!(
        active > 0,
        "the full original Job must include the independently verified live leader"
    );
    assert!(!exited, "End and real EOF must occur before leader exit");
    assert!(
        matches!(pending, Ok(false)),
        "End/EOF incorrectly allowed a live leader: {pending:?}"
    );
    assert!(marker);
    assert_eq!(fixture.active().unwrap(), 0);
    assert!(fixture.leader_exited().unwrap());
    assert!(
        matches!(complete, Ok(true)),
        "natural exit not accepted: {complete:?}"
    );
    assert_eq!(fixture.child.exit_code(), Some(0));
}

#[test]
fn normal_exit_requires_control_closed_and_preserves_real_nonzero_leader_code() {
    let mut fixture = WindowsNormalExitTestSupport::spawn("nonzero-exit").unwrap();
    let result = (|| -> Result<_, ChildError> {
        fixture.await_eof()?;
        let code = fixture.await_leader_exit()?;
        // Job 活动数可晚于 leader 信号更新；仍以同一个原始期限观察。
        while fixture.active()? != 0 {
            fixture
                .check()
                .map_err(|e| ChildError::io("normal accounting wait", e))?;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let deadline = fixture.deadline();
        let pending = fixture.child.poll_normal_exit(|| check(deadline));
        fixture.close_control()?;
        let complete = fixture.child.poll_normal_exit(|| check(deadline));
        let repeated = fixture.child.poll_normal_exit(|| check(deadline));
        Ok((code, pending, complete, repeated))
    })();
    fixture.release_and_observe().unwrap();
    let (code, pending, complete, repeated) = result.unwrap();
    assert_eq!(
        code, 7,
        "normal transport completion must not normalize failure exit code"
    );
    assert_eq!(fixture.active().unwrap(), 0);
    assert!(fixture.leader_exited().unwrap());
    assert!(
        matches!(pending, Ok(false)),
        "open control accepted: {pending:?}"
    );
    assert!(
        matches!(complete, Ok(true)),
        "closed control not accepted: {complete:?}"
    );
    assert!(
        matches!(repeated, Ok(true)),
        "cached fact query changed: {repeated:?}"
    );
    assert_eq!(fixture.child.exit_code(), Some(7));
}

#[test]
fn normal_exit_checkpoint_retains_nonclone_error_without_killing_qualified_job() {
    let mut fixture = WindowsNormalExitTestSupport::spawn("live-leader").unwrap();
    let calls = Cell::new(0);
    let result = (|| -> Result<_, ChildError> {
        fixture.await_eof()?;
        fixture.close_control()?;
        let first = fixture.heartbeat_advances("leader-heartbeat")?;
        fixture.leader_alive()?;
        let active_before = fixture.active()?;
        let exited_before = fixture.leader_exited()?;
        let checkpoint = fixture.child.poll_normal_exit(|| {
            calls.set(calls.get() + 1);
            Err::<(), _>(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "exact normal checkpoint sentinel",
            ))
        });
        let second = fixture.heartbeat_advances("leader-heartbeat")?;
        fixture.leader_alive()?;
        let active_after = fixture.active()?;
        let exited_after = fixture.leader_exited()?;
        fixture.release_and_observe()?;
        let deadline = fixture.deadline();
        let complete = fixture.child.poll_normal_exit(|| check(deadline));
        Ok((
            first,
            second,
            active_before,
            exited_before,
            checkpoint,
            active_after,
            exited_after,
            complete,
        ))
    })();
    fixture.release_and_observe().unwrap();
    let (
        first,
        second,
        active_before,
        exited_before,
        checkpoint,
        active_after,
        exited_after,
        complete,
    ) = result.unwrap();
    assert_eq!(calls.get(), 1);
    match checkpoint {
        Err(ChildSpawnError::Checkpoint {
            primary,
            cleanup: None,
        }) => {
            assert_eq!(primary.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(primary.to_string(), "exact normal checkpoint sentinel");
        }
        other => panic!("checkpoint primary/cleanup changed: {other:?}"),
    }
    assert!(
        second.1 > first.1,
        "heartbeat must continue after checkpoint refusal"
    );
    assert!(
        active_before > 0 && active_after > 0,
        "the full Job must remain nonempty on both sides of checkpoint rejection"
    );
    assert!(
        !exited_before && !exited_after,
        "checkpoint refusal must not kill leader"
    );
    assert!(
        matches!(complete, Ok(true)),
        "natural completion after original error: {complete:?}"
    );
    assert_eq!(fixture.child.exit_code(), Some(0));
    assert_eq!(fixture.active().unwrap(), 0);
    assert!(fixture.natural_marker("leader-natural"));
}
