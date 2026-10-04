use crate::native_admission::NativeAdmission;
use crate::native_job_entry::NativeJobEntry;
use crate::native_jobs::NativeJobs;
use crate::native_worker::NativeWorker;
use crate::{JobHandle, NativeServiceError};
use std::path::PathBuf;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

/// 单会话协调线程登记；managed 记录由独立宿主 manager 回收，订阅者只弱引用。
/// 来源：DiskGraph 原生 Rust PF-06；旧模式不隐式启动无人持有的 manager。
pub(crate) struct NativeLifecycle {
    jobs: Mutex<NativeJobs>,
    changed: Condvar,
    closed: Arc<AtomicBool>,
    limit: usize,
    managed: bool,
}

impl NativeLifecycle {
    /// 建立旧可信会话。参数：closed 为原关闭信号，limit 为并发准入额度；返回：无 manager 状态。
    pub(crate) fn new(closed: Arc<AtomicBool>, limit: usize) -> Self {
        Self {
            jobs: Mutex::new(NativeJobs::default()),
            changed: Condvar::new(),
            closed,
            limit,
            managed: false,
        }
    }

    /// 建立受宿主拥有的会话状态并在启动线程前预留有界队列。
    /// 参数：closed 为原关闭信号，limit 为现有 Engine 额度；返回：状态或分配错误。
    pub(crate) fn managed(
        closed: Arc<AtomicBool>,
        limit: usize,
    ) -> Result<Self, NativeServiceError> {
        let mut state = Self::new(closed, limit);
        state.managed = true;
        state
            .jobs
            .get_mut()
            .unwrap()
            .entries
            .try_reserve(limit)
            .map_err(|_| unavailable("resource_exhausted: native coordinator allocation"))?;
        Ok(state)
    }

    /// 路径 I/O 前登记准入。参数：当前 Arc；返回：自动归还计数的令牌或关闭/额度错误。
    pub(crate) fn admit(self: &Arc<Self>) -> Result<NativeAdmission, NativeServiceError> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        self.ensure_open()?;
        if jobs.in_flight >= self.limit {
            return Err(unavailable("resource_exhausted: native admissions"));
        }
        jobs.in_flight += 1;
        Ok(NativeAdmission {
            lifecycle: self.clone(),
        })
    }

    /// 归还准入并通知 manager/drain。参数：无；返回：无，仅由准入 RAII 调用。
    pub(crate) fn release_admission(&self) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        jobs.in_flight -= 1;
        self.changed.notify_all();
    }

    /// 复验原关闭信号。参数：无；返回：开放或原 session closed 错误。
    pub(crate) fn ensure_open(&self) -> Result<(), NativeServiceError> {
        if self.closed.load(Ordering::SeqCst) {
            Err(unavailable("session closed"))
        } else {
            Ok(())
        }
    }

    /// 原子合并同根活动订阅后再检查 managed 记录额度，启动失败不遗留登记。
    /// 参数：root 为规范根，create 为可失败的真实线程创建；返回：订阅句柄或错误。
    pub(crate) fn register(
        &self,
        root: PathBuf,
        create: impl FnOnce() -> Result<Arc<JobHandle>, NativeServiceError>,
    ) -> Result<Arc<JobHandle>, NativeServiceError> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        self.ensure_open()?;
        Self::reap_records(&mut jobs);
        if let Some(handle) = jobs
            .entries
            .iter()
            .filter(|entry| entry.root == root)
            .find_map(|entry| {
                entry
                    .subscriber
                    .upgrade()
                    .filter(|handle| !handle.is_finished())
            })
        {
            return Ok(handle);
        }
        if self.managed && jobs.entries.len() >= self.limit {
            return Err(unavailable("resource_exhausted: native coordinators"));
        }
        jobs.entries
            .try_reserve(1)
            .map_err(|_| unavailable("resource_exhausted: native coordinator allocation"))?;
        // 队列空间先准备；成功 spawn 后无可失败步骤，句柄先登记再交给宿主/manager。
        let handle = create()?;
        if self.managed {
            handle.worker.manage();
        }
        jobs.entries
            .push(NativeJobEntry::new(root, &handle, self.managed));
        handle.worker.start();
        self.changed.notify_all();
        Ok(handle)
    }

    /// 非阻塞关闭：只发原子信号和通知。参数：无；返回：无，不读数据库或 join。
    pub(crate) fn close(&self) {
        let jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        self.closed.store(true, Ordering::SeqCst);
        for entry in &jobs.entries {
            entry.cancel.store(true, Ordering::SeqCst);
        }
        self.changed.notify_all();
    }

    fn reap_records(jobs: &mut NativeJobs) {
        jobs.entries.retain(|entry| match &entry.worker {
            Some(worker) => !worker.is_joined(),
            // 旧模式只维护活动弱订阅，不以此声称线程已 join。
            None => entry
                .subscriber
                .upgrade()
                .is_some_and(|handle| !handle.is_finished()),
        });
    }

    /// 仅删除真实已 join 的 managed 记录；旧模式清理弱订阅。不执行任何 join。
    /// 参数：无；返回：登记状态或已记录 manager 错误。
    pub(crate) fn reap(&self) -> Result<(), NativeServiceError> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        Self::reap_records(&mut jobs);
        match jobs.failure {
            Some(error) => Err(unavailable(error)),
            None => Ok(()),
        }
    }

    /// 等待实际 worker join 事实；manager 本身由独立宿主另行 finalize。
    /// 参数：deadline 为原绝对期限；返回：完成、期限未完成或明确不支持/失败。
    /// 本调用栈从不 join，且不证明 pinned scanner 的物理退出。
    pub(crate) fn drain_until(&self, deadline: Instant) -> Result<(), NativeServiceError> {
        if !self.managed {
            return Err(unavailable(
                "unsupported: legacy session has no managed coordinator owner",
            ));
        }
        let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            for entry in &jobs.entries {
                if let Some(worker) = &entry.worker {
                    worker.try_join().map_err(unavailable)?;
                }
            }
            if Instant::now() >= deadline {
                return Err(unavailable("coordinator drain deadline exceeded"));
            }
            Self::reap_records(&mut jobs);
            if let Some(error) = jobs.failure {
                return Err(unavailable(error));
            }
            if jobs.in_flight == 0 && jobs.entries.is_empty() {
                return Ok(());
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| unavailable("coordinator drain deadline exceeded"))?;
            jobs = self
                .changed
                .wait_timeout(jobs, remaining)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }

    /// 唯一 manager 主循环。参数：无；返回：关闭且准入/所有 worker 真实退场后结束。
    /// 只在 manager 线程 join；有意保留头部阻塞与额度背压，绝不增加 detached reaper。
    pub(crate) fn run_manager(&self) {
        loop {
            let worker = {
                let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
                loop {
                    Self::reap_records(&mut jobs);
                    if let Some(worker) = jobs.entries.iter().find_map(|entry| entry.worker.clone())
                    {
                        break worker;
                    }
                    if self.closed.load(Ordering::SeqCst) && jobs.in_flight == 0 {
                        return;
                    }
                    jobs = self
                        .changed
                        .wait(jobs)
                        .unwrap_or_else(|error| error.into_inner());
                }
            };
            if let Some(task) = worker.take_task() {
                #[cfg(test)]
                crate::native_manager_tests::after_take();
                let success = task.finish();
                let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
                if !success {
                    jobs.failure = Some("scan worker crashed");
                }
                self.changed.notify_all();
            }
        }
    }

    /// manager 异常时单独记失败并唤醒，未退出 worker 保持 pending。
    /// 参数：无；返回：无，不冒充 worker 的实际 join 事实。
    pub(crate) fn manager_failed(&self) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        jobs.failure = Some("coordinator join manager failed");
        self.closed.store(true, Ordering::SeqCst);
        for entry in &jobs.entries {
            entry.cancel.store(true, Ordering::SeqCst);
            if let Some(worker) = &entry.worker {
                worker.manager_failed();
            }
        }
        self.changed.notify_all();
    }

    /// manager 已被宿主实际 join 后回收异常遗留队列，包括尚未归还的准入。
    /// 参数：无；返回：保留 manager/worker 错误，同时确保所有登记句柄已经真实 join。
    pub(crate) fn finalize_remaining(&self) -> Result<(), NativeServiceError> {
        loop {
            let worker: Arc<NativeWorker> = {
                let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
                Self::reap_records(&mut jobs);
                if let Some(worker) = jobs.entries.iter().find_map(|entry| entry.worker.clone()) {
                    worker
                } else if jobs.in_flight == 0 {
                    return jobs.failure.map_or(Ok(()), |error| Err(unavailable(error)));
                } else {
                    drop(
                        self.changed
                            .wait(jobs)
                            .unwrap_or_else(|error| error.into_inner()),
                    );
                    continue;
                }
            };
            if let Some(task) = worker.take_task() {
                let success = task.finish();
                let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
                if !success {
                    jobs.failure.get_or_insert("scan worker crashed");
                }
                self.changed.notify_all();
            }
        }
    }
}

fn unavailable(reason: &str) -> NativeServiceError {
    NativeServiceError::Unavailable {
        reason: reason.to_owned(),
    }
}
