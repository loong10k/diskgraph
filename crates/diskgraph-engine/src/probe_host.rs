use crate::probe_resource_pool::ProbeResourcePool;
use crate::{EngineError, ProbeRecovery};
use std::sync::Arc;

/// Windows受管理探针的固定容量宿主；来源：PF-06原owner与外部恢复合同，无Java对象。
/// 不授予命令、路径或主体权限；宿主必须另行保留构造时返回的Recovery。
pub struct ProbeHost {
    pub(crate) registry: Arc<ProbeResourcePool>,
}

impl ProbeHost {
    /// 参数：capacity为非零有限session容量，每槽预建一个child容量；返回：宿主与独立恢复责任，尚未创建进程。
    /// Recovery必须放在Engine/session及其catch之外，恢复失败不能丢弃或用重建Host补充容量。
    pub fn new(capacity: u32) -> Result<(Self, ProbeRecovery), EngineError> {
        let registry = ProbeResourcePool::new(capacity)?;
        let recovery = ProbeRecovery::new(Arc::clone(&registry));
        Ok((Self { registry }, recovery))
    }
}
