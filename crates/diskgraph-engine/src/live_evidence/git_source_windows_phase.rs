use super::git_source_windows_diagnostic::GitSourceWindowsDiagnostic;
use std::marker::PhantomData;
use std::rc::Rc;

/// Windows 测试的原采样调用阶段守卫；来源：原生 Rust Git 源调用链。
/// 嵌套及错误退场恢复上层固定标签，不拥有状态或第二个期限。
pub(super) struct GitSourceWindowsPhase {
    previous: Option<&'static str>,
    _thread_bound: PhantomData<Rc<()>>,
}

impl GitSourceWindowsPhase {
    /// 参数：phase 为已明确挂载的源码语义阶段，未知阶段不推断路径来源。
    /// 返回：请求启用时保存原 TLS 阶段的守卫，其余测试保持未启用。
    pub(super) fn new(phase: &'static str) -> Self {
        Self {
            previous: GitSourceWindowsDiagnostic::replace_phase(phase),
            _thread_bound: PhantomData,
        }
    }
}

impl Drop for GitSourceWindowsPhase {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            GitSourceWindowsDiagnostic::restore_phase(previous);
        }
    }
}
