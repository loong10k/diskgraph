use crate::native_join_task::NativeJoinTask;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{JoinHandle, ThreadId};
#[cfg(test)]
mod tests;

/// 协调线程的唯一句柄与真实 join 结果；managed 模式只允许 manager 取走句柄。
/// 来源：DiskGraph 原生 Rust PF-06；不代表 pinned scanner 的物理退出。
pub(crate) struct NativeWorker {
    thread: Mutex<(Option<JoinHandle<()>>, Option<bool>)>,
    joined: Condvar,
    thread_id: ThreadId,
    managed: AtomicBool,
    manager_failed: AtomicBool,
    start: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl NativeWorker {
    /// 接管真实线程。参数：thread 为唯一句柄；返回：尚未 join 的记录。
    pub(crate) fn new(thread: JoinHandle<()>) -> Self {
        Self {
            thread_id: thread.thread().id(),
            thread: Mutex::new((Some(thread), None)),
            joined: Condvar::new(),
            managed: AtomicBool::new(false),
            manager_failed: AtomicBool::new(false),
            start: Mutex::new(None),
        }
    }

    /// 接管等待登记握手的真实线程。参数：thread 为句柄，start 为启动发送端；返回：未放行记录。
    pub(crate) fn with_start(thread: JoinHandle<()>, start: std::sync::mpsc::Sender<()>) -> Self {
        let worker = Self::new(thread);
        *worker.start.lock().unwrap() = Some(start);
        worker
    }

    /// 在注册或旧可信包装接管句柄后放行工作。参数：无；返回：无，失败接收方仍由真实 join 处理。
    pub(crate) fn start(&self) {
        if let Some(start) = self
            .start
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = start.send(());
        }
    }

    /// 发布给订阅者之前转交 manager 模式。参数：无；返回：无，不等待线程。
    pub(crate) fn manage(&self) {
        self.managed.store(true, Ordering::SeqCst);
    }

    /// 等待真实 join；旧模式自行 join，managed 模式只等待 manager 通知。
    /// 参数：无；返回：真实退出结果或 manager 失效。调用者不得持登记/业务锁。
    pub(crate) fn join(&self) -> Result<(), &'static str> {
        self.check_not_self()?;
        let mut state = self
            .thread
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(success) = state.1 {
                return outcome(success);
            }
            if self.manager_failed.load(Ordering::SeqCst) {
                return Err("coordinator join manager failed");
            }
            if !self.managed.load(Ordering::SeqCst)
                && let Some(thread) = state.0.take()
            {
                drop(state);
                self.complete(thread.join().is_ok());
                state = self
                    .thread
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
            } else {
                state = self
                    .joined
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
        }
    }

    /// 只观察真实 join 结果，绝不从 is_finished 推导可同步 join。
    /// 参数：无；返回：真实已 join/尚待退出，或 self-wait/退出错误。
    pub(crate) fn try_join(&self) -> Result<bool, &'static str> {
        self.check_not_self()?;
        match self
            .thread
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .1
        {
            Some(success) => outcome(success).map(|()| true),
            None if self.manager_failed.load(Ordering::SeqCst) => {
                Err("coordinator join manager failed")
            }
            None => Ok(false),
        }
    }

    /// 由唯一 manager 或已 join manager 的宿主接管待处理线程。
    /// 参数：当前 Arc；返回：保留 unwind 回收责任的任务，不执行 join。
    pub(crate) fn take_task(self: &Arc<Self>) -> Option<NativeJoinTask> {
        self.thread
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .0
            .take()
            .map(|thread| NativeJoinTask::new(self.clone(), thread))
    }

    /// 仅在实际 JoinHandle::join 返回后发布结果。参数：success 为 join 结果；返回：无。
    pub(crate) fn complete(&self, success: bool) {
        self.thread
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .1 = Some(success);
        self.joined.notify_all();
    }

    /// manager 失败时唤醒订阅者，不伪造线程已退出。参数：无；返回：无。
    pub(crate) fn manager_failed(&self) {
        let _state = self
            .thread
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.manager_failed.store(true, Ordering::SeqCst);
        self.joined.notify_all();
    }

    /// 查询真实 join 事实。参数：无；返回：成功或失败线程已经 join 时为 true。
    pub(crate) fn is_joined(&self) -> bool {
        self.thread
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .1
            .is_some()
    }

    /// 拒绝当前线程等待自己，即使其他线程已取走句柄。参数：无；返回：资格结果。
    pub(crate) fn check_not_self(&self) -> Result<(), &'static str> {
        if std::thread::current().id() == self.thread_id {
            Err("coordinator cannot join its own thread")
        } else {
            Ok(())
        }
    }
}

fn outcome(success: bool) -> Result<(), &'static str> {
    if success {
        Ok(())
    } else {
        Err("scan worker crashed")
    }
}
