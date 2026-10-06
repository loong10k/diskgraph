use crate::live_evidence::git_private_directory_owner::GitPrivateDirectoryOwner;
use crate::probe_resource_pool::ProbeResourcePool;
use crate::scan_worker_registry::ScanWorkerRegistry;
use std::sync::Arc;

/// 私有目录借用与同session槽的唯一绑定；来源：PF-06，目录借出期间不释放代次。
pub(crate) struct ProbeDirectoryBinding {
    pool: Arc<ProbeResourcePool>,
    index: usize,
    generation: u64,
    inner: Arc<ScanWorkerRegistry>,
    returned: bool,
}
impl ProbeDirectoryBinding {
    /// 参数：已标记借用的原槽；返回：唯一binding，不持OS或数据库锁。
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
            returned: false,
        }
    }
    /// 参数：原目录payload；返回：显式清理结果，child未完成或删除失败则无分配交还同owner。
    pub(crate) fn complete(
        &mut self,
        owner: &mut Option<GitPrivateDirectoryOwner>,
    ) -> Result<(), String> {
        if self.returned {
            return if owner.is_some() {
                Ok(())
            } else {
                Err("private Git owner retained for recovery".into())
            };
        }
        let result = match self.inner.occupied() {
            Ok(0) => owner.as_mut().ok_or("private Git owner absent")?.cleanup(),
            Ok(_) => Err("private Git cleanup deferred until original child recovery".into()),
            Err(error) => Err(format!("private Git child recovery state: {error}")),
        };
        let retained = if result.is_err() { owner.take() } else { None };
        self.pool
            .return_directory(self.index, self.generation, retained);
        self.returned = true;
        result
    }
}
impl Drop for ProbeDirectoryBinding {
    fn drop(&mut self) {
        if !self.returned {
            self.pool
                .return_directory(self.index, self.generation, None);
        }
    }
}
