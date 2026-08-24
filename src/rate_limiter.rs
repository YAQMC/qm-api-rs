//! 令牌桶限流器 (对应 Python 端 `niquests.AsyncTokenBucketLimiter`).

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::error::{QmError, Result};

const DEFAULT_RATE: f64 = 10.0;
const DEFAULT_CAPACITY: f64 = 50.0;

/// 异步令牌桶限流器.
#[derive(Debug)]
pub struct TokenBucket {
    rate: f64,
    capacity: f64,
    state: Arc<Mutex<TokenBucketState>>,
}

#[derive(Debug)]
struct TokenBucketState {
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    fn from_valid(rate: f64, capacity: f64) -> Self {
        TokenBucket {
            rate,
            capacity,
            state: Arc::new(Mutex::new(TokenBucketState {
                tokens: capacity,
                last: Instant::now(),
            })),
        }
    }

    /// 创建令牌桶.
    ///
    /// - `rate`: 每秒补充的令牌数 (即每秒允许的请求数).
    /// - `capacity`: 桶容量, 允许的突发请求数.
    ///
    /// 为保持现有 `new() -> Self` API 的兼容性，非法配置（非有限值、`rate <= 0`
    /// 或 `capacity < 1`）会回退到安全默认值。需要严格校验并感知配置错误的调用方
    /// 应使用 [`Self::try_new`].
    pub fn new(rate: f64, capacity: f64) -> Self {
        let rate = if rate.is_finite() && rate > 0.0 {
            rate
        } else {
            DEFAULT_RATE
        };
        let capacity = if capacity.is_finite() && capacity >= 1.0 {
            capacity
        } else {
            DEFAULT_CAPACITY
        };
        Self::from_valid(rate, capacity)
    }

    /// 严格创建令牌桶；非法参数返回错误而不是永久等待或触发浮点 Duration panic.
    pub fn try_new(rate: f64, capacity: f64) -> Result<Self> {
        if !rate.is_finite() || rate <= 0.0 {
            return Err(QmError::ValueError(
                "TokenBucket rate 必须是大于 0 的有限数".into(),
            ));
        }
        if !capacity.is_finite() || capacity < 1.0 {
            return Err(QmError::ValueError(
                "TokenBucket capacity 必须是至少 1 的有限数".into(),
            ));
        }
        Ok(Self::from_valid(rate, capacity))
    }

    /// 获取一个令牌; 若桶为空则等待直至补充.
    pub async fn acquire(&self) {
        loop {
            let mut state = self.state.lock().await;
            let now = Instant::now();
            let elapsed = now.duration_since(state.last).as_secs_f64();
            state.tokens = (state.tokens + elapsed * self.rate).min(self.capacity);
            state.last = now;

            if state.tokens >= 1.0 {
                state.tokens -= 1.0;
                return;
            }

            // 构造时已保证 rate 是有限正数；这里再限制 sleep 上界和下界，避免未来
            // 状态损坏时把非有限值传给 Duration::from_secs_f64.
            let needed = ((1.0 - state.tokens) / self.rate).clamp(0.0, 1.0);
            drop(state);
            tokio::time::sleep(Duration::from_secs_f64(needed)).await;
        }
    }
}

impl Clone for TokenBucket {
    fn clone(&self) -> Self {
        TokenBucket {
            rate: self.rate,
            capacity: self.capacity,
            state: Arc::clone(&self.state),
        }
    }
}

impl Default for TokenBucket {
    fn default() -> Self {
        Self::from_valid(DEFAULT_RATE, DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_constructor_rejects_invalid_values() {
        for rate in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(TokenBucket::try_new(rate, 1.0).is_err());
        }
        for capacity in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(TokenBucket::try_new(1.0, capacity).is_err());
        }
    }

    #[tokio::test]
    async fn compatibility_constructor_never_hangs_on_invalid_values() {
        let bucket = TokenBucket::new(f64::NAN, -1.0);
        tokio::time::timeout(Duration::from_millis(100), bucket.acquire())
            .await
            .expect("invalid compatibility configuration must fall back safely");
    }
}
