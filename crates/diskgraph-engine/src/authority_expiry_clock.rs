use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

thread_local! {
    static NOW: Cell<Option<u64>> = const { Cell::new(None) };
}

/// 当前测试线程的授权墙钟覆盖；退场恢复原值，不改变生产时钟或单调查询期限。
pub(crate) struct AuthorityExpiryClock {
    previous: Option<u64>,
    // 守卫不能移到另一线程，否则Drop会恢复错误线程的墙钟。
    thread: PhantomData<Rc<()>>,
}

impl AuthorityExpiryClock {
    /// 参数：seconds为确定性Unix秒；返回：恢复当前线程原覆盖的守卫。
    pub(crate) fn install(seconds: u64) -> Self {
        Self {
            previous: NOW.with(|now| now.replace(Some(seconds))),
            thread: PhantomData,
        }
    }

    /// 参数：seconds为同次请求推进后的墙钟秒；返回：无，不修改token到期值。
    pub(crate) fn advance(&self, seconds: u64) {
        NOW.with(|now| now.set(Some(seconds)));
    }

    /// 参数：无；返回：当前测试线程覆盖值，None沿用真实系统墙钟。
    pub(crate) fn now() -> Option<u64> {
        NOW.with(Cell::get)
    }
}

impl Drop for AuthorityExpiryClock {
    fn drop(&mut self) {
        NOW.with(|now| now.set(self.previous));
    }
}

#[cfg(test)]
mod tests {
    use super::AuthorityExpiryClock;

    #[test]
    fn nested_clock_restores_and_other_threads_keep_real_clock() {
        assert_eq!(AuthorityExpiryClock::now(), None);
        let outer = AuthorityExpiryClock::install(99);
        assert_eq!(
            std::thread::spawn(AuthorityExpiryClock::now)
                .join()
                .unwrap(),
            None
        );
        {
            let inner = AuthorityExpiryClock::install(100);
            inner.advance(101);
            assert_eq!(AuthorityExpiryClock::now(), Some(101));
        }
        assert_eq!(AuthorityExpiryClock::now(), Some(99));
        drop(outer);
        assert_eq!(AuthorityExpiryClock::now(), None);
    }

    #[test]
    fn clock_restores_after_unwind() {
        let result = std::panic::catch_unwind(|| {
            let _clock = AuthorityExpiryClock::install(99);
            panic!("controlled clock unwind");
        });
        assert!(result.is_err());
        assert_eq!(AuthorityExpiryClock::now(), None);
    }
}
