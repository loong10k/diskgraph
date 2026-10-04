use diskgraph_core::ProcessEvidenceFailureCode as Failure;

/// 借用原生工作回调的类型名；来源：Rust 扫描桥接，保留原 dyn Fn 的高阶借用推导。
/// 生命周期限定捕获的路径/句柄借用，不要求工作闭包为静态对象。
pub(super) type NativeWork<'a, T> =
    dyn Fn(&dyn Fn() -> Result<(), Failure>) -> Result<T, Failure> + 'a;
