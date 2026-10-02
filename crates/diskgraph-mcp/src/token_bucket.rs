/// 单个客户端的请求额度及最后一次有效观测时间。
/// 来源：DiskGraph 原生 Rust HTTP 限流实现，无对应 Java 对象。
pub(crate) struct TokenBucket {
    pub(crate) tokens: f64,
    pub(crate) last_refill_ms: u128,
}
