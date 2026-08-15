//! 令牌桶限流器 (对应 Python 端 `niquests.AsyncTokenBucketLimiter`).

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

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
    /// 创建令牌桶.
    ///
    /// - `rate`: 每秒补充的令牌数 (即每秒允许的请求数).
    /// - `capacity`: 桶容量, 允许的突发请求数.
    pub fn new(rate: f64, capacity: f64) -> Self {
        TokenBucket {
            rate,
            capacity,
            state: Arc::new(Mutex::new(TokenBucketState {
                tokens: capacity,
                last: Instant::now(),
            })),
        }
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
            let needed = (1.0 - state.tokens) / self.rate;
            drop(state);
            tokio::time::sleep(Duration::from_secs_f64(needed.min(1.0))).await;
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
        // 与参考客户端一致: 10 请求/秒, 突发容量 50.
        TokenBucket::new(10.0, 50.0)
    }
}
