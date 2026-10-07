//! Background job runner (P4 task 5.8, specs MCP-05 / RT-01): executes queued
//! durable jobs independently of any connection. The runner is what makes the
//! connection/job separation real: a client that disconnects after requesting
//! an index loses nothing, because the job's lifecycle lives in the control
//! store, not in the socket that asked for it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::{Engine, EngineError};
use diskgraph_store::JobRecord;

/// Poll interval for newly queued jobs. Short enough that tests and agents
/// see prompt progress, long enough to keep idle deployments quiet.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Runs queued jobs to completion, one claim at a time per engine.
///
/// The runner is deliberately separate from `Engine::open`: engine-level tests
/// hold jobs queued on purpose (quota tests), so nothing runs them unless a
/// server or an explicit test asks for it.
/// 持有后台 runner 的生命周期与协作停止状态。
/// 来源：原生 Rust diskgraph-engine::JobRunner。
pub struct JobRunner {
    engine: Arc<Engine>,
    stop: Arc<AtomicBool>,
    owner: String,
    require_authority: bool,
    worker: Option<JoinHandle<()>>,
    #[cfg(windows)]
    probe_recovery: Option<Arc<crate::ProbeRecovery>>,
}

impl JobRunner {
    /// Starts the runner thread. Keep the returned handle alive for as long
    /// as jobs should progress; dropping it stops subsequent scheduling without
    /// blocking on any scan already running.
    /// 参数：engine 为已有共享引擎。
    /// 返回：后台调度句柄；原实现保留启动线程失败时无 worker 的行为。
    pub fn start(engine: Arc<Engine>) -> Self {
        Self::start_mode(
            engine,
            false,
            #[cfg(windows)]
            None,
        )
    }

    /// 启动只执行来源明确任务的远程宿主 runner。
    /// 参数：engine 为共享引擎；返回：同一调度 owner 的生命周期句柄。
    /// 无请求授权记录的历史任务将失败，不隐式借用宿主本机身份。
    pub fn start_strict(engine: Arc<Engine>) -> Self {
        Self::start_mode(
            engine,
            true,
            #[cfg(windows)]
            None,
        )
    }

    /// 使用外部宿主保留的同一探针恢复责任启动调度器，不重建容量或授予权限。
    /// 来源：原生 Rust PF-06；参数：engine 为共享引擎，require_authority 为远程严格模式，
    /// recovery 为宿主在协议 catch 外持有的同一对象；返回：每次认领前单次恢复的 runner。
    #[cfg(windows)]
    pub fn start_with_probe_recovery(
        engine: Arc<Engine>,
        require_authority: bool,
        recovery: Arc<crate::ProbeRecovery>,
    ) -> Result<Self, EngineError> {
        if !engine
            .probe_host
            .as_ref()
            .is_some_and(|host| recovery.belongs_to(host))
        {
            return Err(diskgraph_core::BusinessError::InvalidArgument.into());
        }
        Ok(Self::start_mode(engine, require_authority, Some(recovery)))
    }

    fn start_mode(
        engine: Arc<Engine>,
        require_authority: bool,
        #[cfg(windows)] probe_recovery: Option<Arc<crate::ProbeRecovery>>,
    ) -> Self {
        let owner = format!("runner-{}", uuid::Uuid::new_v4());
        let stop = Arc::new(AtomicBool::new(false));
        let worker_engine = Arc::clone(&engine);
        let worker_stop = Arc::clone(&stop);
        let worker_owner = owner.clone();
        #[cfg(windows)]
        let worker_recovery = probe_recovery.as_ref().map(Arc::clone);
        let worker = std::thread::Builder::new()
            .name("diskgraph-job-runner".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::SeqCst) {
                    // 运行期沿用显式宿主的同一Recovery，一轮一次，不补充Host或吞清理失败。
                    #[cfg(windows)]
                    if let Some(recovery) = &worker_recovery {
                        match recovery.drain() {
                            Ok(true) => {}
                            Ok(false) | Err(_) => {
                                // 尚有原owner时暂停新认领，避免把容量不足误判成任务失败。
                                std::thread::sleep(POLL_INTERVAL);
                                continue;
                            }
                        }
                    }
                    if let Err(error) = run_one_queued(
                        &worker_engine,
                        &worker_owner,
                        &worker_stop,
                        require_authority,
                    ) {
                        // Claim races are routine: whoever claimed first wins.
                        // Real failures stay visible on stderr.
                        eprintln!("diskgraph job runner: {error}");
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
            })
            .ok();
        Self {
            engine,
            stop,
            owner,
            require_authority,
            worker,
            #[cfg(windows)]
            probe_recovery,
        }
    }

    /// Stops the runner at the next poll boundary and waits for the thread.
    /// 参数：消费当前 runner。
    /// 返回：等待后台线程退出后返回；已开始的扫描按既有预算/租约结束。
    /// 后台线程 panic 时继续原 payload；需先恢复资源的宿主使用 stop_and_join。
    pub fn stop(self) {
        if let Err(payload) = self.stop_and_join() {
            std::panic::resume_unwind(payload);
        }
    }

    /// 停止并返回原后台 panic，让宿主先恢复资源再继续异常。
    /// 来源：原生 Rust PF-06；参数：消费 runner；返回：真实 join 结果及未经替换的 panic payload。
    pub fn stop_and_join(mut self) -> std::thread::Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        match self.worker.take() {
            Some(worker) => worker.join(),
            None => Ok(()),
        }
    }

    /// The fencing owner this runner claims jobs under (diagnostics/tests).
    /// 参数：无。
    /// 返回：本 runner 用于认领任务的 owner 标识。
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Claims and runs at most one queued job. Exposed for tests that want a
    /// deterministic single tick instead of the polling thread.
    /// 参数：无。
    /// 返回：至多一个任务的执行记录、无可执行任务的 None 或执行错误。
    pub fn tick(&self) -> Result<Option<JobRecord>, EngineError> {
        #[cfg(windows)]
        if let Some(recovery) = &self.probe_recovery
            && !recovery.drain()?
        {
            return Ok(None);
        }
        run_one_queued(
            &self.engine,
            &self.owner,
            &self.stop,
            self.require_authority,
        )
    }
}

impl Drop for JobRunner {
    fn drop(&mut self) {
        // 关闭宿主 handle 后在下一次认领检查停止；已通过检查的扫描按预算/租约完成。
        // Drop 不等待长扫描，空闲 worker 在下个 100ms 调度边界释放 Engine。
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn run_one_queued(
    engine: &Arc<Engine>,
    owner: &str,
    stop: &AtomicBool,
    require_authority: bool,
) -> Result<Option<JobRecord>, EngineError> {
    if stop.load(Ordering::SeqCst) {
        return Ok(None);
    }
    // 同Engine受管理runner/tick不得在检查容量后并发认领，忙时保持任务Queued。
    // 仅调度路径try_lock，查询不进入此锁；graph/control/OS恢复仍沿用原各自锁顺序。
    #[cfg(windows)]
    let _admission = if engine.probe_host.is_some() {
        match engine.runner_admission.try_lock() {
            Ok(guard) => Some(guard),
            Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
            // 此锁只保护互斥资格，没有可被panic破坏的业务数据；取回资格后仍复核原池。
            Err(std::sync::TryLockError::Poisoned(error)) => Some(error.into_inner()),
        }
    } else {
        None
    };
    #[cfg(windows)]
    if let Some(host) = &engine.probe_host {
        // 前一任务可能在外层drain和本轮准入之间移交失败owner；持准入后复核原池。
        if host.registry.occupied()? != 0 {
            return Ok(None);
        }
    }
    // 每轮只准入一页；失效请求由逐项认领落终态，不先解码或清理全队列权限。
    let queued = engine.queued_jobs_limited(64)?;
    for job in queued {
        // 队列读取可能等待控制锁；返回后及每次竞争失败后的认领都重新检查停止。
        if stop.load(Ordering::SeqCst) {
            return Ok(None);
        }
        #[cfg(windows)]
        if let Some(host) = &engine.probe_host {
            // 前一候选可能执行后因fencing失败continue且移交owner；每次新认领都复核。
            if host.registry.occupied()? != 0 {
                return Ok(None);
            }
        }
        let outcome = if require_authority {
            engine.run_job_strict(&job.job_id, owner)
        } else {
            engine.run_job(&job.job_id, owner)
        };
        match outcome {
            Ok(record) => return Ok(Some(record)),
            // A competing runner may win any candidate. Continue to the next
            // eligible job rather than starving the rest of the queue.
            Err(EngineError::Store(diskgraph_store::StoreError::Conflict(_)))
            | Err(EngineError::Store(diskgraph_store::StoreError::StaleOwner)) => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
