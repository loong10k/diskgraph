use diskgraph_engine::{Engine, EngineError};
use diskgraph_store::JobRecord;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

/// 外层协调线程唯一拥有的 Engine runner；提前返回或 unwind 请求同代停止并真实 join。
/// 来源：DiskGraph 原生 Rust PF-06；仅 Engine 成功 claim 后绑定 owner/fence，绝不按裸 job ID 取消。
pub(crate) struct NativeRunnerGuard {
    thread: Option<JoinHandle<Result<JobRecord, EngineError>>>,
    request_cancel: Arc<AtomicBool>,
    deny_stop: Arc<AtomicBool>,
}

impl NativeRunnerGuard {
    /// 启动真实 runner 并保留两份原始信号 Arc。
    /// 参数：engine/job 为拟认领任务，request_cancel 是宿主请求，deny_stop 只来自实际拒权；返回：唯一 guard 或启动错误。
    pub(crate) fn spawn(
        engine: Arc<Engine>,
        job: String,
        request_cancel: Arc<AtomicBool>,
        deny_stop: Arc<AtomicBool>,
        #[cfg(test)] hook: Option<crate::native_runner_exit_tests::RunnerHook>,
    ) -> Result<Self, String> {
        let request = request_cancel.clone();
        let denied = deny_stop.clone();
        let thread = std::thread::Builder::new()
            .name("diskgraph-native-runner".into())
            .spawn(move || {
                #[cfg(test)]
                if let Some(hook) = &hook {
                    hook("runner_start");
                }
                let outcome = engine.run_job_with_stop_signals(&job, "ffi-worker", request, denied);
                #[cfg(test)]
                if let Some(hook) = &hook {
                    hook("runner_outcome");
                }
                outcome
            })
            .map_err(|error| format!("scan runner spawn failed: {error}"))?;
        Ok(Self {
            thread: Some(thread),
            request_cancel,
            deny_stop,
        })
    }

    /// 检查是否仍有本机 runner。参数：无；返回：真实句柄是否仍归本 guard。
    pub(crate) fn is_pending(&self) -> bool {
        self.thread.is_some()
    }

    /// 只用于允许阻塞的外层协调线程决定何时收取结果，不作为物理退场证明。
    /// 参数：无；返回：runner 主体已经结束的提示，实际结果仍须 join。
    pub(crate) fn is_finished(&self) -> bool {
        self.thread.as_ref().is_some_and(JoinHandle::is_finished)
    }

    /// 正常收取 runner；不无故设置任何停止信号。
    /// 参数：无；返回：真实 join 的 Engine 结果；已取走为 None，恐慌保持原错误。
    pub(crate) fn join(&mut self) -> Result<Option<Result<JobRecord, EngineError>>, String> {
        self.thread
            .take()
            .map(|thread| thread.join().map_err(|_| "scan worker crashed".to_owned()))
            .transpose()
    }
}

impl Drop for NativeRunnerGuard {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            // 拒权单独传递；内部 keeper 停止不能推论 deny，晚信号不能改已提交事实。
            if !self.deny_stop.load(Ordering::SeqCst) {
                self.request_cancel.store(true, Ordering::SeqCst);
            }
            // guard 只在外层协调线程上，且调用点不持有 registry、JobState 或 DB 锁。
            let _ = thread.join();
        }
    }
}
