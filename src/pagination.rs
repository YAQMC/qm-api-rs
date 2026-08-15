//! 连续翻页组件 (对应 Python 端 `core/pagination.py`).
//!
//! 提供跨页拉取与数据项展开的通用分页器.

use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::Result;

/// 页面抓取函数: 接收当前请求参数, 返回解析后的单页响应.
pub type FetchFn<T> =
    Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<T>> + Send>> + Send + Sync>;

/// 下一页参数构建函数: 根据当前请求参数与上一页响应构造下一页参数.
pub type NextParamsFn<T> = Arc<dyn Fn(&Value, &T) -> Option<Value> + Send + Sync>;

/// 通用连续翻页器.
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
    /// 创建分页器.
    ///
    /// - `initial`: 首页请求参数.
    /// - `fetch`: 页面抓取函数 (接收当前页参数, 返回 `Result<T>`).
    /// - `next_params`: 根据响应计算下一页参数; 返回 `None` 表示没有更多页.
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

    /// 设置最大拉取页数限制.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// 是否还有更多页.
    pub fn has_more(&self) -> bool {
        match self.limit {
            Some(limit) => self.yielded < limit && self.current_params.is_some(),
            None => self.current_params.is_some(),
        }
    }

    /// 拉取并返回下一页响应; 没有更多页时返回 `None`.
    pub async fn next(&mut self) -> Option<Result<T>> {
        let params = self.current_params.take()?;
        let result = (self.fetch)(params.clone()).await;
        match &result {
            Ok(resp) => {
                self.current_params = (self.next_params)(&params, resp);
                self.yielded += 1;
                if let Some(limit) = self.limit {
                    if self.yielded >= limit {
                        self.current_params = None;
                    }
                }
            }
            Err(_) => {
                self.current_params = None;
            }
        }
        Some(result)
    }

    /// 收集所有页的响应为列表.
    pub async fn collect(&mut self) -> Result<Vec<T>> {
        let mut out = Vec::new();
        while let Some(result) = self.next().await {
            out.push(result?);
        }
        Ok(out)
    }

    /// 跨页收集数据项为列表.
    ///
    /// `extract` 从单页响应中提取条目.
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
/// 每次请求将 `page_key` 递增 1; `has_more` 返回 `false` 时停止.
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
        let mut next = params.clone();
        next[page_key] = Value::from(page + 1);
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
/// 每次请求将 `offset_key` 加上 `page_size_key` 的值; `has_more` 返回 `false` 时停止.
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
        let mut next = params.clone();
        next[offset_key] = Value::from(offset + step);
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
}
