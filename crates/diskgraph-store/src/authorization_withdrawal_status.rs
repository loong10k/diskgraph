/// 当前请求的已提交负向事实或代次失效，不包含任何授予权限的状态。
/// 来源：DiskGraph 原生 Rust D45 请求级撤权契约。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationWithdrawalStatus {
    /// 没有适用的已知撤权；调用方仍必须执行原实时授权与期限检查。
    Unchanged,
    /// 本请求依赖的权限或 scope 已被可信入口成功持久撤销。
    Withdrawn,
    /// 传入连接不再是请求的原代次；调用方必须失败关闭，不能移交到新库继续授权。
    Invalidated,
}
