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
    /// 参数：原目录payload、retry_retained 表示显式重试；返回：真实清理结果，失败交还同owner。
    /// Drop 必须传 false；只有同代次原会话仍活跃时才能重新借出已移交目录。
    pub(crate) fn complete(
        &mut self,
        owner: &mut Option<GitPrivateDirectoryOwner>,
        retry_retained: bool,
    ) -> Result<(), String> {
        if self.returned {
            if owner.is_some() {
                return Ok(());
            }
            if !retry_retained {
                return Err("private Git owner retained for recovery".into());
            }
            // 显式重试只重新借出原代次原 owner；失败仍由原恢复池持有。
            *owner = Some(
                self.pool
                    .reborrow_retained_directory(self.index, self.generation)
                    .map_err(|error| format!("private Git owner retained for recovery: {error}"))?,
            );
            self.returned = false;
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
