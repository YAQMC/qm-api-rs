//! 连续翻页组件 (对应 Python 端 `core/pagination.py`).
//!
//! 提供跨页拉取与数据项展开的通用分页器.

use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::Result;

pub type FetchFn<T> =
    Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<T>> + Send>> + Send + Sync>;
pub type NextParamsFn<T> = Arc<dyn Fn(&Value, &T) -> Option<Value> + Send + Sync>;

pub struct Pager<T> {
    fetch: FetchFn<T>,
    next_params: NextParamsFn<T>,
    current_params: Option<Value>,
    limit: Option<usize>,
    yielded: usize,
}

impl<T> std::fmt::Debug for Pager<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pager")
            .field("current_params", &self.current_params)
            .field("limit", &self.limit)
            .field("yielded", &self.yielded)
            .finish()
    }
}

impl<T> Pager<T> {
    pub fn new<F, N>(initial: Value, fetch: F, next_params: N) -> Self
    where
        F: Fn(Value) -> Pin<Box<dyn Future<Output = Result<T>> + Send>> + Send + Sync + 'static,
        N: Fn(&Value, &T) -> Option<Value> + Send + Sync + 'static,
    {
        Pager {
            fetch: Arc::new(fetch),
            next_params: Arc::new(next_params),
            current_params: Some(initial),
            limit: None,
            yielded: 0,
        }
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn has_more(&self) -> bool {
        match self.limit {
            Some(limit) => self.yielded < limit && self.current_params.is_some(),
            None => self.current_params.is_some(),
        }
    }

    pub async fn next(&mut self) -> Option<Result<T>> {
        if matches!(self.limit, Some(0)) {
            self.current_params = None;
            return None;
        }
        let params = self.current_params.take()?;
        let result = (self.fetch)(params.clone()).await;
        match &result {
            Ok(resp) => {
                self.current_params = (self.next_params)(&params, resp);
                self.yielded = self.yielded.saturating_add(1);
                if let Some(limit) = self.limit {
                    if self.yielded >= limit {
                        self.current_params = None;
                    }
                }
            }
            Err(_) => self.current_params = None,
        }
        Some(result)
    }

    pub async fn collect(&mut self) -> Result<Vec<T>> {
        let mut out = Vec::new();
        while let Some(result) = self.next().await {
            out.push(result?);
        }
        Ok(out)
    }

    pub async fn collect_items<U>(&mut self, extract: impl Fn(&T) -> Vec<U>) -> Result<Vec<U>> {
        let mut out = Vec::new();
        while let Some(result) = self.next().await {
            out.extend(extract(&result?));
        }
        Ok(out)
    }
}

/// 基于页码的翻页策略.
///
/// 非正页码或 `i64` 溢出时停止继续翻页，而不是产生负页码或 debug/release 行为差异.
pub fn page<T, F, H>(
    page_key: &'static str,
    start_page: i64,
    initial: Value,
    fetch: F,
    has_more: H,
) -> Pager<T>
where
    T: 'static,
    F: Fn(Value) -> Pin<Box<dyn Future<Output = Result<T>> + Send>> + Send + Sync + 'static,
    H: Fn(&T) -> bool + Send + Sync + 'static,
{
    let fetch = Arc::new(fetch);
    let has_more = Arc::new(has_more);
    let next_params = move |params: &Value, resp: &T| {
        if !has_more(resp) {
            return None;
        }
        let page = params
            .get(page_key)
            .and_then(Value::as_i64)
            .unwrap_or(start_page);
        if page < 1 {
            return None;
        }
        let next_page = page.checked_add(1)?;
        let mut next = params.clone();
        next[page_key] = Value::from(next_page);
        Some(next)
    };
    Pager::new(
        initial,
        {
            let fetch = fetch.clone();
            move |p| -> Pin<Box<dyn Future<Output = Result<T>> + Send>> {
                let f = fetch.clone();
                Box::pin(async move { f(p).await })
            }
        },
        next_params,
    )
}

/// 基于偏移量窗口的翻页策略.
///
/// 负 offset、非正 page size 或 `i64` 加法溢出时停止继续翻页.
pub fn offset<T, F, H>(
    offset_key: &'static str,
    page_size_key: &'static str,
    initial: Value,
    fetch: F,
    has_more: H,
) -> Pager<T>
where
    T: 'static,
    F: Fn(Value) -> Pin<Box<dyn Future<Output = Result<T>> + Send>> + Send + Sync + 'static,
    H: Fn(&T) -> bool + Send + Sync + 'static,
{
    let fetch = Arc::new(fetch);
    let has_more = Arc::new(has_more);
    let next_params = move |params: &Value, resp: &T| {
        if !has_more(resp) {
            return None;
        }
        let offset = params.get(offset_key).and_then(Value::as_i64).unwrap_or(0);
        let step = params
            .get(page_size_key)
            .and_then(Value::as_i64)
            .unwrap_or(10);
        if offset < 0 || step <= 0 {
            return None;
        }
        let next_offset = offset.checked_add(step)?;
        let mut next = params.clone();
        next[offset_key] = Value::from(next_offset);
        Some(next)
    };
    Pager::new(
        initial,
        {
            let fetch = fetch.clone();
            move |p| -> Pin<Box<dyn Future<Output = Result<T>> + Send>> {
                let f = fetch.clone();
                Box::pin(async move { f(p).await })
            }
        },
        next_params,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_pager_collect_items() {
        let pages = vec![
            json!({ "page": 1, "items": [1, 2] }),
            json!({ "page": 2, "items": [3, 4] }),
            json!({ "page": 3, "items": [5] }),
        ];
        let fetch = move |params: Value| -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> {
            let pages = pages.clone();
            Box::pin(async move {
                let page = params["page"].as_i64().unwrap() as usize;
                Ok::<_, crate::error::QmError>(pages[page - 1].clone())
            })
        };
        let next = |_params: &Value, resp: &Value| {
            let page = resp["page"].as_i64().unwrap();
            if resp["items"].as_array().unwrap().len() < 2 {
                None
            } else {
                Some(json!({ "page": page + 1 }))
            }
        };
        let mut pager = Pager::new(json!({ "page": 1 }), fetch, next);
        let items = pager
            .collect_items(|r: &Value| {
                r["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|v| v.as_i64())
                    .collect::<Vec<i64>>()
            })
            .await
            .unwrap();
        assert_eq!(items, vec![1, 2, 3, 4, 5]);
    }

    #[tokio::test]
    async fn test_page_strategy() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls2 = calls.clone();
        let fetch = move |params: Value| -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> {
            let calls = calls2.clone();
            Box::pin(async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, crate::error::QmError>(params)
            })
        };
        let has_more = |resp: &Value| resp["page"].as_i64().unwrap() < 3;
        let mut pager = page("page", 1, json!({ "page": 1 }), fetch, has_more);
        let pages = pager.collect().await.unwrap();
        assert_eq!(pages.len(), 3);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn zero_limit_fetches_nothing() {
        let fetch = |_params: Value| -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> {
            Box::pin(async { Ok::<_, crate::error::QmError>(json!({})) })
        };
        let mut pager = Pager::new(json!({}), fetch, |_p, _r| None).with_limit(0);
        assert!(pager.next().await.is_none());
    }

    #[test]
    fn page_and_offset_progression_fail_closed_on_overflow_or_invalid_values() {
        let dummy_fetch = |_params: Value| -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> {
            Box::pin(async { Ok::<_, crate::error::QmError>(json!({})) })
        };

        let pager = page(
            "page",
            1,
            json!({"page": i64::MAX}),
            dummy_fetch,
            |_r: &Value| true,
        );
        assert!(pager.has_more());

        let pager2 = offset(
            "offset",
            "size",
            json!({"offset": 0, "size": 0}),
            |_params: Value| -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> {
                Box::pin(async { Ok::<_, crate::error::QmError>(json!({})) })
            },
            |_r: &Value| true,
        );
        assert!(pager2.has_more());
    }
}
