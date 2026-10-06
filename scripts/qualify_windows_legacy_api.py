"""仅给冻结旧行为源码补充当前测试所需的 调用接口；原语行为不变，使用必须可观测。

旧绑定直接委托其真实兼容清理，不实现新期限合同。原生资格必须拒绝
非目标测试意外执行该绑定；期限资格则用它证明旧兼容调用忽略期限。
"""
import re

MARKER = "DG_LEGACY_RECOVERY_API_DELEGATE_USED=1"
SEAL_MARKER = "DG_LEGACY_SUPERVISOR_SEAL_UNSUPPORTED=1"
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
    """补真实旧清理委托；旧版本的关闭准入明确拒绝，禁止作为新行为证据。"""
    if MARKER.encode() in baseline or SEAL_MARKER.encode() in baseline:
        raise RuntimeError("legacy recovery binding is already instrumented")
    bindings = [ADAPTERS[kind]]
    if kind == "pool":
        bindings.append(("seal_admission", '''\nimpl ProbeResourcePool {
    /// 冻结旧版本不支持关闭准入；拒绝而不伪造新能力。
    pub(crate) fn seal_admission(&self) -> Result<(), EngineError> {
        eprintln!("DG_LEGACY_SUPERVISOR_SEAL_UNSUPPORTED=1");
        Err(diskgraph_core::BusinessError::Unsupported.into())
    }
}
'''.encode()))
    adapted = baseline
    for method, adapter in bindings:
        pattern = rb"\bfn\s+" + method.encode() + rb"\s*\("
        current_count = len(re.findall(pattern, candidate))
        old_count = len(re.findall(pattern, baseline))
        if current_count > 1 or old_count > 1:
            raise RuntimeError("legacy recovery binding method is ambiguous")
        if current_count == 1 and old_count == 0:
            adapted += adapter
    return adapted


POOL_DEADLINE_SUPPORT = (
    "crates/diskgraph-engine/src/live_evidence/git_private_directory_owner.rs",
    "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup.rs",
)

def strip_unused_pool_deadline_method(source):
    """旧池没有该调用边；仅移除不可达的新方法，兼容清理正文逐字保留。"""
    count = len(re.findall(rb"\bfn\s+cleanup_until\s*\(", source))
    if count == 0:
        return source
    pattern = (rb"(?m)^    ///[^\r\n]*\r?\n(?:    ///[^\r\n]*\r?\n)*"
               rb"(?:    #\[cfg\(windows\)\]\r?\n)?"
               rb"    pub\((?:crate|super)\) fn cleanup_until\([\s\S]*?^    }\r?\n")
    matches = list(re.finditer(pattern, source))
    if count != 1 or len(matches) != 1:
        raise RuntimeError("unreachable deadline method boundary is not unique")
    match = matches[0]
    method = match.group()
    if (len(re.findall(rb"\bfn\s+", method)) != 1
            or method.count(b"{") != method.count(b"}")):
        raise RuntimeError("unreachable deadline method boundary is malformed")
    return source[:match.start()] + source[match.end():]

def prepare_pool_deadline_support(root):
    """返回当前支持源码及旧池不可达方法剔除后的副本；调用方须在 finally 逐字恢复。"""
    original = {name: (root / name).read_bytes() for name in POOL_DEADLINE_SUPPORT}
    adapted = {name: strip_unused_pool_deadline_method(data) for name, data in original.items()}
    return original, adapted
