//! Darwin真实成员查询竞态；来源：libproc与PF-06，回调只改变真实子进程，不替代系统查询。
use super::macos_child_group::normal_view;
use super::macos_group_view::MacosGroupView;
use super::unix_normal_exit_test_support as fixture;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

thread_local! {
    static SAMPLES: Cell<usize> = const { Cell::new(0) };
    static TARGET: Cell<i32> = const { Cell::new(0) };
    static QUERIES: Cell<usize> = const { Cell::new(0) };
    static BEFORE_QUERY: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

/// 参数：pid为即将真实查询的组成员；返回：无，只在同一线程原调用点交接一次回调。
pub(super) fn before_member_query(pid: i32) {
    if TARGET.get() == pid {
        QUERIES.set(QUERIES.get() + 1);
        if let Some(callback) = BEFORE_QUERY.with(|slot| slot.borrow_mut().take()) {
            callback();
        }
    }
}

/// 参数：无；返回：无，仅记录真实组采集的调用次数，不替换系统返回。
pub(super) fn before_group_sample() {
    SAMPLES.set(SAMPLES.get() + 1);
}

fn before(pid: i32, callback: impl FnOnce() + 'static) {
    TARGET.set(pid);
    QUERIES.set(0);
    BEFORE_QUERY.with(|slot| *slot.borrow_mut() = Some(Box::new(callback)));
}

fn wait_absent(pid: i32) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let bytes = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                1,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                std::mem::size_of::<libc::proc_bsdinfo>() as i32,
            )
        };
        if bytes == 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "original fixture member did not disappear"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn naturally_disappeared_member_requires_a_fresh_complete_view() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("descendant", directory.path());
    let pid = fixture::qualified_identity(directory.path());
    fixture::wait_file(&directory.path().join("descendant_ready"));
    let leaf = directory.path().join("leaf");
    let member = fixture::scalar(&leaf, "pid");
    child.request_control_close().unwrap();
    fixture::drain(&mut child, true);
    fixture::retained(pid);
    before(member, move || {
        fixture::release(&leaf);
        wait_absent(member);
    });
    assert!(matches!(normal_view(pid as u32), MacosGroupView::AllExited));
    assert_eq!(
        QUERIES.get(),
        1,
        "fresh sample must not reuse the removed member"
    );
    fixture::retained(pid);
    assert!(child.poll_normal_exit(|| Ok::<(), ()>(())).unwrap());
    fixture::reaped(pid);
}

#[test]
fn lost_retained_leader_is_unknown_without_a_second_query() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("exit", directory.path());
    let pid = fixture::qualified_identity(directory.path());
    child.request_control_close().unwrap();
    fixture::drain(&mut child, true);
    fixture::retained(pid);
    before(pid, move || {
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    });
    assert!(matches!(
        normal_view(pid as u32),
        MacosGroupView::Unknown(_)
    ));
    assert_eq!(QUERIES.get(), 1, "leader loss must not be retried");
    assert!(child.poll().is_err());
    fixture::reaped(pid);
}

/// 两次真实采样各失去一个普通成员，第三份完整视图不得绕过本次有限重采额度。
#[test]
fn second_disappeared_member_remains_unknown_without_a_third_sample() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("two_descendants", directory.path());
    let pid = fixture::qualified_identity(directory.path());
    fixture::wait_file(&directory.path().join("descendant_ready"));
    let first = directory.path().join("leaf");
    let second = directory.path().join("leaf2");
    let first_pid = fixture::scalar(&first, "pid");
    let second_pid = fixture::scalar(&second, "pid");
    // 按真实PID排序安排消失顺序，不假设系统PID分配连续或leader数值最小。
    let (first, first_pid, second, second_pid) = if first_pid < second_pid {
        (first, first_pid, second, second_pid)
    } else {
        (second, second_pid, first, first_pid)
    };
    child.request_control_close().unwrap();
    fixture::drain(&mut child, true);
    fixture::retained(pid);
    SAMPLES.set(0);
    before(first_pid, move || {
        fixture::release(&first);
        wait_absent(first_pid);
        before(second_pid, move || {
            fixture::release(&second);
            wait_absent(second_pid);
        });
    });
    assert!(matches!(
        normal_view(pid as u32),
        MacosGroupView::Unknown(_)
    ));
    assert_eq!(
        SAMPLES.get(),
        2,
        "third sample must not consume another native query"
    );
    fixture::retained(pid);
    // 第二次未知仍保留原owner；后续独立调用重新核验真实组，才允许消费leader。
    assert!(child.poll_normal_exit(|| Ok::<(), ()>(())).unwrap());
    fixture::reaped(pid);
}
