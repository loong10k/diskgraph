use crate::EngineError;
use crate::probe_directory_binding::ProbeDirectoryBinding;
use crate::probe_resource_pool::ProbeResourcePool;
use crate::scan_worker_registry::ScanWorkerRegistry;
use std::sync::Arc;

/// 每个ProbeBudget独占的session代次；来源：PF-06，活跃期间不能由Recovery释放容量。
pub(crate) struct ProbeSessionLease {
    pool: Arc<ProbeResourcePool>,
    index: usize,
    generation: u64,
    inner: Arc<ScanWorkerRegistry>,
}
impl ProbeSessionLease {
    /// 参数：已预留槽及原inner；返回：唯一lease，不再次预留容量。
    pub(crate) fn new(
        pool: Arc<ProbeResourcePool>,
        index: usize,
        generation: u64,
        inner: Arc<ScanWorkerRegistry>,
    ) -> Self {
        Self {
            pool,
            index,
            generation,
            inner,
        }
    }
    /// 参数：无；返回：同槽child registry，不重建预算或清理责任。
    pub(crate) fn inner(&self) -> Arc<ScanWorkerRegistry> {
        Arc::clone(&self.inner)
    }
    /// 参数：无；返回：唯一目录binding，创建目录之前取得。
    pub(crate) fn directory(&self) -> Result<ProbeDirectoryBinding, EngineError> {
        self.pool.borrow_directory(self.index, self.generation)?;
        Ok(ProbeDirectoryBinding::new(
            Arc::clone(&self.pool),
            self.index,
            self.generation,
            Arc::clone(&self.inner),
        ))
    }
}
impl Drop for ProbeSessionLease {
    fn drop(&mut self) {
        let empty = matches!(self.inner.occupied(), Ok(0));
        self.pool.finish_session(self.index, self.generation, empty);
    }
}
