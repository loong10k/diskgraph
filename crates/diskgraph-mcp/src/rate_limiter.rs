use std::sync::Mutex;
use std::time::Instant;

use crate::http::HttpLimits;
use crate::rate_limit_state::RateLimitState;
use crate::token_bucket::TokenBucket;

const MAX_CLIENTS: usize = 4_096;
const MAX_CLIENT_KEY_BYTES: usize = 64;
const IDLE_RETENTION_MS: u128 = 60_000;
const SWEEP_INTERVAL_MS: u128 = 1_000;
const CAPACITY_RETRY_MS: u64 = 1_000;

/// 跨连接共享的客户端令牌桶，最多保留 4096 个不超过 64 字节的键。
/// 来源：DiskGraph 原生 Rust HTTP 限流实现，无对应 Java 对象。
pub struct RateLimiter {
    inner: Mutex<RateLimitState>,
    refill_per_second: f64,
    burst: f64,
    now_ms: Box<dyn Fn() -> u128 + Send + Sync>,
}

impl RateLimiter {
    /// 创建共享限流器；参数 `limits` 提供每客户端每秒额度，零值按一处理。
    /// 返回使用单调时钟的空限流器；桶容量与每秒补充额度相同。
    pub fn new(limits: &HttpLimits) -> Self {
        let burst = f64::from(limits.max_requests_per_second_per_client).max(1.0);
        let started = Instant::now();
        Self {
            inner: Mutex::new(RateLimitState::default()),
            refill_per_second: burst,
            burst,
            now_ms: Box::new(move || started.elapsed().as_millis()),
        }
    }

    /// 注入测试时钟；参数 `clock` 返回毫秒观测，返回替换时钟后的限流器。
    #[cfg(test)]
    pub(crate) fn with_clock(mut self, clock: impl Fn() -> u128 + Send + Sync + 'static) -> Self {
        self.now_ms = Box::new(clock);
        self
    }

    /// 消耗客户端的一份额度；参数 `client` 为跨连接稳定且有界的客户端键。
    /// 返回成功或重试毫秒数；非法键、满表及锁失效也拒绝，不重置活跃桶。
    pub fn check(&self, client: &str) -> Result<(), u64> {
        self.check_at(client, (self.now_ms)())
    }

    /// 按显式观测消耗额度；参数 `client` 为客户端键，`now_ms` 为时钟毫秒数。
    /// 返回成功或重试毫秒数；旧观测按共享水位钳制，不能重复补充额度。
    pub(crate) fn check_at(&self, client: &str, now_ms: u128) -> Result<(), u64> {
        if client.is_empty() || client.len() > MAX_CLIENT_KEY_BYTES {
            return Err(CAPACITY_RETRY_MS);
        }
        let mut state = self.inner.lock().map_err(|_| CAPACITY_RETRY_MS)?;
        // 时钟在锁外读取；并发请求的锁获取次序不一定等于时钟观测次序。
        let now_ms = now_ms.max(state.last_observed_ms);
        state.last_observed_ms = now_ms;
        if now_ms.saturating_sub(state.last_sweep_ms) >= SWEEP_INTERVAL_MS {
            // 仅回收已闲置一分钟的桶。补满最多需要一秒，回收不增加有效额度；
            // 洪流满表时也最多每秒执行一次有界扫描，不逐出仍活跃的客户端。
            state.buckets.retain(|_, bucket| {
                now_ms.saturating_sub(bucket.last_refill_ms) < IDLE_RETENTION_MS
            });
            state.last_sweep_ms = now_ms;
        }
        if !state.buckets.contains_key(client) {
            if state.buckets.len() >= MAX_CLIENTS {
                return Err(CAPACITY_RETRY_MS);
            }
            state.buckets.insert(
                client.to_owned(),
                TokenBucket {
                    tokens: self.burst,
                    last_refill_ms: now_ms,
                },
            );
        }
        // 已存在的键只借用查找，避免为每次请求重新分配 String。
        let bucket = state.buckets.get_mut(client).expect("client was inserted");
        let elapsed_ms = now_ms.saturating_sub(bucket.last_refill_ms);
        bucket.tokens = (bucket.tokens + (elapsed_ms as f64 / 1_000.0) * self.refill_per_second)
            .min(self.burst);
        bucket.last_refill_ms = now_ms;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let retry_ms = ((1.0 - bucket.tokens) / self.refill_per_second * 1_000.0).ceil() as u64;
            Err(retry_ms.max(1))
        }
    }
}
