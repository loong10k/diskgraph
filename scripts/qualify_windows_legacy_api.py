"""仅给冻结旧行为源码补充当前测试所需的 调用接口；原语行为不变，使用必须可观测。

旧绑定直接委托其真实兼容清理，不实现新期限合同。原生资格必须拒绝
非目标测试意外执行该绑定；期限资格则用它证明旧兼容调用忽略期限。
"""
import re

MARKER = "DG_LEGACY_RECOVERY_API_DELEGATE_USED=1"
ADAPTERS = {
    "pool": ("drain_until", '''\nimpl ProbeResourcePool {
    /// 仅冻结旧行为的测试 调用接口：原 drain 实际执行，不能作为有界清理证明。
    pub(crate) fn drain_until(&self, _deadline: std::time::Instant) -> Result<bool, EngineError> {
        eprintln!("DG_LEGACY_RECOVERY_API_DELEGATE_USED=1");
        self.drain()
    }
}
'''.encode()),
    "directory": ("cleanup_until", '''\nimpl WindowsGitCleanup {
    /// 仅冻结旧行为的测试 调用接口：原 cleanup 实际执行，不能作为有界清理证明。
    pub(super) fn cleanup_until(
        &mut self,
        capacity: Option<&GitPrivateCapacity>,
        _deadline: std::time::Instant,
    ) -> Result<bool, String> {
        eprintln!("DG_LEGACY_RECOVERY_API_DELEGATE_USED=1");
        self.cleanup(capacity).map(|()| true)
    }
}
'''.encode()),
}


def bridge_baseline(candidate, baseline, kind):
    """当前调用方需要新方法且旧版本没有时，补真实旧行为委托；已存在方法拒绝重复绑定。"""
    method, adapter = ADAPTERS[kind]
    pattern = rb"\bfn\s+" + method.encode() + rb"\s*\("
    current_count = len(re.findall(pattern, candidate))
    old_count = len(re.findall(pattern, baseline))
    if current_count > 1 or old_count > 1:
        raise RuntimeError("legacy recovery binding method is ambiguous")
    if MARKER.encode() in baseline:
        raise RuntimeError("legacy recovery binding is already instrumented")
    if current_count == 0 or old_count == 1:
        return baseline
    return baseline + adapter
