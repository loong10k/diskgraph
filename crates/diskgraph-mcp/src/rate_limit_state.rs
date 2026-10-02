use std::collections::HashMap;

use crate::token_bucket::TokenBucket;

/// 同一服务共享的有界客户端额度及单调时间水位。
/// 来源：DiskGraph 原生 Rust HTTP 限流实现，无对应 Java 对象。
#[derive(Default)]
pub(crate) struct RateLimitState {
    pub(crate) buckets: HashMap<String, TokenBucket>,
    pub(crate) last_sweep_ms: u128,
    pub(crate) last_observed_ms: u128,
}
