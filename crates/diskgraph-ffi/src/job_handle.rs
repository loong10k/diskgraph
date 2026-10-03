// 原生后台扫描句柄的轮询、授权和协作取消。

/// A live scan job handle (P7 task 9.2, PF-01). `spawn_scan_json` returns
/// immediately, so a UI thread never blocks on a walk: progress is polled,
/// cancellation is cooperative, and `result_json` is the only joining call.
/// 释放最后一个句柄请求协作取消，后台线程仍会完成清理并记录终态。
#[cfg_attr(doc, doc = "来源：DiskGraph 原生 Rust UniFFI 绑定；无 Java 对应对象。")]
#[derive(uniffi::Object)]
pub struct JobHandle {
    pub(crate) cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) state: std::sync::Arc<std::sync::Mutex<JobState>>,
}

#[uniffi::export]
impl JobHandle {
    /// 读取进度而不等待扫描完成；实时授权访问控制库，宿主应在后台线程调用。
    #[cfg_attr(
        doc,
        doc = "读取进度而不等待扫描完成；实时授权访问控制库，宿主应在后台线程调用。\n读取当前进度并重新核对实时权限。\n参数：无额外输入，使用本句柄共享状态。\n返回：running/finished 进度 envelope，不等待扫描结束。"
    )]
    pub fn progress_json(&self) -> String {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (state_name, data): (&str, Option<serde_json::Value>) = if state.finished {
            ("finished", None)
        } else {
            ("running", None)
        };
        let answer = serde_json::json!({
            "state": state_name,
            "result": data,
            "progress": state.progress,
        });
        let authorization = state.authorization.clone();
        drop(state);
        job_response(authorization, Ok(answer))
    }

    /// 不等待扫描完成的结果轮询；运行中返回 None，权限检查应由宿主后台线程执行。
    #[cfg_attr(
        doc,
        doc = "不等待扫描完成的结果轮询；运行中返回 None，权限检查应由宿主后台线程执行。\n非阻塞轮询作业结果并重新核对权限。\n参数：无额外输入，使用本句柄共享状态。\n返回：运行中为 None，完成或授权失败为对应 JSON envelope。"
    )]
    pub fn poll_result_json(&self) -> Option<String> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let authorization = state.authorization.clone();
        let finished = state.finished;
        let result = state.result.clone();
        drop(state);
        if let Some(check) = &authorization
            && let Err(error) = check()
        {
            return Some(response(Err(error)));
        }
        if !finished {
            return None;
        }
        Some(job_response(
            authorization,
            result.unwrap_or_else(|| Err("job finished without a result".into())),
        ))
    }

    /// Asks the walk to stop at its next observation boundary. A job that
    /// already finished is unaffected; cancellation is cooperative, so the
    /// final state stays truthful about how far the walk got.
    #[cfg_attr(
        doc,
        doc = "Asks the walk to stop at its next observation boundary. A job that\nalready finished is unaffected; cancellation is cooperative, so the\nfinal state stays truthful about how far the walk got.\n请求扫描在下一个观测边界协作停止。\n参数：无额外输入，设置当前句柄取消标志。\n返回：无；已完成作业不重新执行。"
    )]
    pub fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Joins the worker and returns the same envelope the synchronous call
    /// would have produced. Idempotent: later calls return the recorded
    /// result without re-running anything.
    #[cfg_attr(
        doc,
        doc = "Joins the worker and returns the same envelope the synchronous call\nwould have produced. Idempotent: later calls return the recorded\nresult without re-running anything.\n等待作业状态完成并返回持久结果。\n参数：无额外输入，重复调用读取同一共享结果。\n返回：同步调用形态的 JSON envelope，返回前再次授权。"
    )]
    pub fn result_json(&self) -> String {
        loop {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.finished {
                let result = state
                    .result
                    .clone()
                    .unwrap_or_else(|| Err("job finished without a result".into()));
                let authorization = state.authorization.clone();
                drop(state);
                return job_response(authorization, result);
            }
            drop(state);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

fn job_response(authorization: Option<JobAuthorization>, result: ApiResult) -> String {
    if let Some(check) = authorization
        && let Err(error) = check()
    {
        return response(Err(error));
    }
    response(result)
}

impl JobHandle {
    /// 检查共享作业是否已经完成。
    /// 参数：无额外输入，读取句柄状态锁。
    /// 返回：完成为 true；不等待 worker 结束。
    pub(crate) fn is_finished(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .finished
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
