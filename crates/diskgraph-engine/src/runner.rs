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
pub struct JobRunner {
    engine: Arc<Engine>,
    stop: Arc<AtomicBool>,
    owner: String,
    worker: Option<JoinHandle<()>>,
}

impl JobRunner {
    /// Starts the runner thread. Keep the returned handle alive for as long
    /// as jobs should progress; dropping it detaches the thread.
    pub fn start(engine: Arc<Engine>) -> Self {
        let owner = format!("runner-{}", uuid::Uuid::new_v4());
        let stop = Arc::new(AtomicBool::new(false));
        let worker_engine = Arc::clone(&engine);
        let worker_stop = Arc::clone(&stop);
        let worker_owner = owner.clone();
        let worker = std::thread::Builder::new()
            .name("diskgraph-job-runner".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::SeqCst) {
                    if let Err(error) = run_one_queued(&worker_engine, &worker_owner) {
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
            worker,
        }
    }

    /// Stops the runner at the next poll boundary and waits for the thread.
    pub fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker {
            let _ = worker.join();
        }
    }

    /// The fencing owner this runner claims jobs under (diagnostics/tests).
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Claims and runs at most one queued job. Exposed for tests that want a
    /// deterministic single tick instead of the polling thread.
    pub fn tick(&self) -> Result<Option<JobRecord>, EngineError> {
        run_one_queued(&self.engine, &self.owner)
    }
}

fn run_one_queued(engine: &Arc<Engine>, owner: &str) -> Result<Option<JobRecord>, EngineError> {
    let queued = engine.queued_jobs()?;
    let Some(job) = queued.first() else {
        return Ok(None);
    };
    match engine.run_job(&job.job_id, owner) {
        Ok(record) => Ok(Some(record)),
        // Another runner (or a --wait CLI) claimed it first: not an error.
        Err(EngineError::Store(diskgraph_store::StoreError::Conflict(_)))
        | Err(EngineError::Store(diskgraph_store::StoreError::StaleOwner)) => Ok(None),
        Err(error) => Err(error),
    }
}
