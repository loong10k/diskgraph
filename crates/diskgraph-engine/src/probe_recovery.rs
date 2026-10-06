use crate::EngineError;
use crate::probe_resource_pool::ProbeResourcePool;
use std::sync::Arc;

/// Windows探针唯一外部恢复责任；来源：PF-06，不与扫描镜像资格绑定，无Java对象。
#[must_use = "retain probe recovery outside catch_unwind until original owners are reaped"]
pub struct ProbeRecovery {
    registry: Arc<ProbeResourcePool>,
}

impl ProbeRecovery {
    /// 参数：registry 为原宿主共享资源池；返回：绑定同一资源池的恢复入口，不新增容量。
    pub(crate) fn new(registry: Arc<ProbeResourcePool>) -> Self {
        Self { registry }
    }

    /// 参数：host 为 Engine 已绑定的宿主；返回：是否属于原同一资源池，拒绝错配恢复责任。
    pub(crate) fn belongs_to(&self, host: &crate::ProbeHost) -> bool {
        Arc::ptr_eq(&self.registry, &host.registry)
    }

    /// 参数：无；返回：原registry实际占用数，包括预留、执行及未恢复owner。
    pub fn occupied_slots(&self) -> Result<usize, EngineError> {
        self.registry.occupied()
    }

    /// 按同一绝对期限轮询原进程与私有目录；来源：PF-06 宿主恢复合同。
    /// 参数：deadline 为调用方期限；返回：真实清理完毕为 true，到期或竞争为 false，原错误为 Err。
    /// 未完成时必须继续持有本恢复责任；不刷新期限，不表示进程可以丢弃 owner 后退出。
    /// 原状态归还锁及单次内核调用仍不承诺硬墙钟上限。
    pub fn drain_until(&self, deadline: std::time::Instant) -> Result<bool, EngineError> {
        self.registry.drain_until(deadline)
    }

    /// 参数：无；返回：全部原owner真实回收时true、仍活动时false、失败时原错误。
    /// 一轮尝试失败保留同owner；不启动后台reaper，不自动无限重试，内核I/O仍可能阻塞。
    pub fn drain(&self) -> Result<bool, EngineError> {
        self.registry.drain()
    }
}
