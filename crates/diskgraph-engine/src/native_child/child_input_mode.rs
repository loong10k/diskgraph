/// 子进程标准输入的显式内部模式；来源：原生 Rust 扫描进程与只读探针契约。
/// 模式只决定管道所有权，不授予执行权限，也不创建请求预算。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChildInputMode {
    /// 原有探针的空输入模式；保持原启动检查与句柄白名单。
    Null,
    /// 扫描 worker 的独占非阻塞控制输入，不使用宿主全局信号策略。
    WorkerControl,
}
