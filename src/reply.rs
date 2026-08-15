//! 固定 CGI 响应类型 (transport contract).
//!
//! transport 层 (`ApiContext::request_cgi` / `request_cgi_batch`) 永远返回
//! `CgiReply<T>` (固定形状 `{ code, data }`), 不再依赖
//! `allow_error_codes` / `parse_on_allow` 之类按配置改变返回结构的选项.
//!
//! 业务层负责解释 `code`:
//! - 普通接口: `require_success()` 在 `code != 0` 时映射为错误并取出 `data`;
//! - 登录等需要解释特殊状态码的接口: 直接读取 `code` 字段自行处理.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{QmError, Result};

/// 固定形状的 CGI 响应: `{ code, data }`.
#[derive(Debug, Clone, PartialEq)]
pub struct CgiReply<T> {
    /// 业务错误码 (`req_N.code`), `0` 表示成功.
    pub code: i64,
    /// 业务响应数据 (`req_N.data`).
    pub data: T,
}

impl<T> CgiReply<T> {
    /// 构造响应.
    pub const fn new(code: i64, data: T) -> Self {
        CgiReply { code, data }
    }

    /// 业务错误码.
    pub const fn code(&self) -> i64 {
        self.code
    }
}

/// 将业务错误码映射为 `QmError` (不含网络/全局信封错误).
///
/// `data` 在进入错误前会做脱敏 (截断 + 掩码敏感令牌), 避免完整响应进日志.
pub(crate) fn map_cgi_code(code: i64, data: &Value) -> QmError {
    let redacted = crate::error::redact_payload(&data.to_string(), 400);
    match code {
        2000 => QmError::SignatureRequired,
        2001 => QmError::RateLimited,
        1000 | 104401 | 104400 => QmError::CredentialExpired(format!("code {code}")),
        _ => QmError::CgiApi {
            code,
            data: redacted,
        },
    }
}

impl CgiReply<Value> {
    /// 是否成功 (`code == 0`).
    pub fn succeeded(&self) -> bool {
        self.code == 0
    }

    /// 是否失败 (`code != 0`).
    pub fn failed(&self) -> bool {
        !self.succeeded()
    }

    /// 将业务错误码映射为 `QmError` (不做抛掷).
    pub fn error(&self) -> QmError {
        map_cgi_code(self.code, &self.data)
    }

    /// 默认业务处理: `code == 0` 时返回 `data`, 否则映射为错误.
    pub fn require_success(self) -> Result<Value> {
        if self.code == 0 {
            Ok(self.data)
        } else {
            Err(map_cgi_code(self.code, &self.data))
        }
    }

    /// 允许部分业务错误码透传数据: `code == 0` 或 `code ∈ allowed` 时返回 `data`.
    ///
    /// 用于诸如"曲谱不存在 (10007)"之类本身携带有效数据的业务状态码.
    pub fn require_success_allowing(self, allowed: &[i64]) -> Result<Value> {
        if self.code == 0 || allowed.contains(&self.code) {
            Ok(self.data)
        } else {
            Err(map_cgi_code(self.code, &self.data))
        }
    }

    /// 反序列化 `data` 为 `T` (仅在 `code == 0` 时).
    ///
    /// `data` 为 `null` 时视为 malformed 而非成功 (不再伪装成空对象).
    pub fn into_typed<T: DeserializeOwned>(self) -> Result<T> {
        let data = self.require_success()?;
        if data.is_null() {
            return Err(QmError::ApiData(
                "CGI 响应成功但 data 为 null (malformed), 请检查接口协议".into(),
            ));
        }
        serde_json::from_value(data).map_err(QmError::from)
    }
}

/// 批量请求的执行报告 (支持部分失败).
///
/// 写歌单、批量收藏等接口常出现部分成功/部分失败, 返回本报告而非整体报错,
/// 由调用方决定如何向用户呈现或重试失败项.
#[derive(Debug, Clone, Default)]
pub struct BatchReport {
    /// 请求总数.
    pub total: usize,
    /// 成功项 (`code == 0`) 的数量.
    pub succeeded: usize,
    /// 失败项: `(请求序号, 业务错误码)`.
    pub failures: Vec<(usize, i64)>,
}

impl BatchReport {
    /// 是否全部成功.
    ///
    /// 空批次 (`total == 0`) **不算**成功, 避免调用方把"无请求"误判为"全部成功".
    pub fn is_ok(&self) -> bool {
        self.total > 0 && self.failures.is_empty()
    }

    /// 失败项的序号列表.
    pub fn failed_indices(&self) -> Vec<usize> {
        self.failures.iter().map(|(i, _)| *i).collect()
    }
}

impl CgiReply<Value> {
    /// 将批量响应按成功 / 失败分组, 保留原始顺序与错误码.
    ///
    /// 返回 `(成功项, 失败项)`, 便于调用方处理部分失败而不整体报错.
    pub fn partition(
        replies: Vec<CgiReply<Value>>,
    ) -> (Vec<CgiReply<Value>>, Vec<CgiReply<Value>>) {
        let mut ok = Vec::new();
        let mut err = Vec::new();
        for reply in replies {
            if reply.succeeded() {
                ok.push(reply);
            } else {
                err.push(reply);
            }
        }
        (ok, err)
    }

    /// 汇总批量结果, 返回 [`BatchReport`] (不抛错, 保留全部失败信息).
    pub fn report(replies: &[CgiReply<Value>]) -> BatchReport {
        let total = replies.len();
        let mut failures = Vec::new();
        for (i, reply) in replies.iter().enumerate() {
            if reply.failed() {
                failures.push((i, reply.code));
            }
        }
        let succeeded = total - failures.len();
        BatchReport {
            total,
            succeeded,
            failures,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_success_ok() {
        let reply = CgiReply::new(0, json_lit());
        let data = reply.require_success().unwrap();
        assert_eq!(data["name"], "周杰伦");
    }

    #[test]
    fn require_success_errors_on_business_code() {
        let reply = CgiReply::new(404, json_lit());
        match reply.require_success() {
            Err(QmError::CgiApi { code, .. }) => assert_eq!(code, 404),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn require_success_maps_special_codes() {
        assert!(matches!(
            CgiReply::new(2000, Value::Null).require_success(),
            Err(QmError::SignatureRequired)
        ));
        assert!(matches!(
            CgiReply::new(2001, Value::Null).require_success(),
            Err(QmError::RateLimited)
        ));
        assert!(matches!(
            CgiReply::new(104401, Value::Null).require_success(),
            Err(QmError::CredentialExpired(_))
        ));
    }

    #[test]
    fn require_success_allowing_passes_allowed_code() {
        let reply = CgiReply::new(10007, json_lit());
        let data = reply.require_success_allowing(&[10007]).unwrap();
        assert_eq!(data["name"], "周杰伦");
    }

    #[test]
    fn require_success_allowing_errors_on_other_code() {
        let reply = CgiReply::new(2001, Value::Null);
        assert!(matches!(
            reply.require_success_allowing(&[10007]),
            Err(QmError::RateLimited)
        ));
    }

    #[test]
    fn partition_splits_success_and_failure() {
        let replies = vec![
            CgiReply::new(0, Value::Null),
            CgiReply::new(2001, Value::Null),
            CgiReply::new(0, Value::Null),
        ];
        let (ok, err) = CgiReply::partition(replies);
        assert_eq!(ok.len(), 2);
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].code, 2001);
    }

    #[test]
    fn report_tracks_partial_failures() {
        let replies = vec![
            CgiReply::new(0, Value::Null),
            CgiReply::new(10007, Value::Null),
            CgiReply::new(0, Value::Null),
        ];
        let report = CgiReply::report(&replies);
        assert_eq!(report.total, 3);
        assert_eq!(report.succeeded, 2);
        assert_eq!(report.failures, vec![(1, 10007)]);
        assert!(!report.is_ok());
        assert_eq!(report.failed_indices(), vec![1]);
    }

    #[test]
    fn report_all_success_is_ok() {
        let replies = vec![CgiReply::new(0, Value::Null), CgiReply::new(0, Value::Null)];
        let report = CgiReply::report(&replies);
        assert!(report.is_ok());
        assert!(report.failures.is_empty());
    }

    #[test]
    fn null_data_is_not_success_for_typed() {
        let reply = CgiReply::new(0, Value::Null);
        assert!(matches!(
            reply.into_typed::<serde_json::Value>(),
            Err(QmError::ApiData(_))
        ));
        // 但 require_success() 仍允许调用方自行处理 null (如返回 Value 的透传).
        let reply = CgiReply::new(0, Value::Null);
        assert!(reply.require_success().unwrap().is_null());
    }

    fn json_lit() -> Value {
        serde_json::json!({ "name": "周杰伦" })
    }
}
