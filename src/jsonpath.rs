//! 极简 JSONPath 提取器 (支持 `$`, `.key` 与 `[*]` 段).
//!
//! 对应 Python 端 `jsonpath_ng` 的用法子集, 用于从 API 响应中抽取嵌套字段.

use serde_json::Value;

/// 解析后的路径段.
#[derive(Debug, Clone)]
enum Segment {
    Key(String),
    Star, // [*]
}

/// 解析形如 `$.a.b[*].c` 的表达式.
fn parse(path: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let trimmed = path.trim_start();
    let rest = trimmed.strip_prefix('$').unwrap_or(trimmed);
    let mut current = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => {
                if !current.is_empty() {
                    segments.push(Segment::Key(std::mem::take(&mut current)));
                }
            }
            '[' => {
                if !current.is_empty() {
                    segments.push(Segment::Key(std::mem::take(&mut current)));
                }
                // 读取到 ']' 为止, 期望 [*]
                let mut inner = String::new();
                while let Some(&n) = chars.peek() {
                    if n == ']' {
                        chars.next();
                        break;
                    }
                    inner.push(n);
                    chars.next();
                }
                if inner.trim() == "*" {
                    segments.push(Segment::Star);
                }
            }
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        segments.push(Segment::Key(current));
    }
    segments
}

fn get_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map.get(key),
        Value::Array(items) => {
            let mut out = None;
            for item in items {
                if let Some(v) = get_key(item, key) {
                    if out.is_none() {
                        out = Some(v);
                    }
                }
            }
            out
        }
        _ => None,
    }
}

/// 递归收集符合剩余路径段的所有值.
fn collect(value: &Value, segments: &[Segment], out: &mut Vec<Value>) {
    match segments.first() {
        None => out.push(value.clone()),
        Some(Segment::Key(key)) => {
            if let Some(next) = get_key(value, key) {
                collect(next, &segments[1..], out);
            }
        }
        Some(Segment::Star) => match value {
            Value::Array(items) => {
                for item in items {
                    collect(item, &segments[1..], out);
                }
            }
            other => collect(other, &segments[1..], out),
        },
    }
}

/// 按表达式提取值.
///
/// - 表达式含 `[*]` 时始终返回数组;
/// - 否则单值返回标量, 多值返回数组.
pub fn extract<'a>(root: &'a Value, expr: &str) -> Value {
    let segments = parse(expr);
    let mut out = Vec::new();
    collect(root, &segments, &mut out);
    if expr.contains("[*]") {
        Value::Array(out)
    } else if out.len() == 1 {
        out.pop().unwrap()
    } else {
        Value::Array(out)
    }
}

/// 提取并按 `T` 反序列化; 缺失或类型不符时返回 `T::default()`.
///
/// 仅用于**明确确认可容忍缺失**的字段 (lenient)。关键字段应使用
/// [`extract_optional`] / [`extract_strict`], 避免 schema drift 被静默吞掉.
pub fn extract_typed<T: serde::de::DeserializeOwned + Default>(root: &Value, expr: &str) -> T {
    let value = extract(root, expr);
    if value.is_null() {
        return T::default();
    }
    serde_json::from_value(value).unwrap_or_default()
}

/// 提取并按 `T` 反序列化; 缺失 / 类型不符时返回 `None` (不吞掉, 上层可见).
#[allow(dead_code)] // 供 `jsonpath_model!` 的 `optional(...)` 策略使用.
pub fn extract_optional<T: serde::de::DeserializeOwned>(root: &Value, expr: &str) -> Option<T> {
    let value = extract(root, expr);
    if value.is_null() {
        return None;
    }
    serde_json::from_value(value).ok()
}

/// 提取并按 `T` 反序列化; 缺失 / 类型不符时返回错误 (严格, 暴露 schema drift).
pub fn extract_strict<T: serde::de::DeserializeOwned>(
    root: &Value,
    expr: &str,
) -> std::result::Result<T, String> {
    let value = extract(root, expr);
    if value.is_null() {
        return Err(format!("JSONPath {expr:?} 提取结果为空"));
    }
    serde_json::from_value(value).map_err(|e| format!("JSONPath {expr:?} 解析失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_basic() {
        let v = json!({"info": {"company": {"content": [1, 2]}}});
        assert_eq!(extract(&v, "$.info.company.content"), json!([1, 2]));
    }

    #[test]
    fn test_extract_star() {
        let v = json!({"vecPlaylistNew": [{"playlists": [{"id": 1}, {"id": 2}]}, {"playlists": [{"id": 3}]}]});
        let got = extract(&v, "$.vecPlaylistNew[*].playlists[*]");
        assert_eq!(got, json!([{"id": 1}, {"id": 2}, {"id": 3}]));
    }

    #[test]
    fn test_extract_missing() {
        let v = json!({});
        assert_eq!(extract(&v, "$.a.b"), Value::Array(vec![]));
    }

    #[test]
    fn test_extract_typed_lenient_default() {
        let v = json!({});
        assert_eq!(extract_typed::<i64>(&v, "$.a.b"), 0);
    }

    #[test]
    fn test_extract_optional_none_on_missing() {
        let v = json!({});
        assert_eq!(extract_optional::<i64>(&v, "$.a.b"), None);
        assert_eq!(extract_optional::<i64>(&json!({"a": {"b": 7}}), "$.a.b"), Some(7));
    }

    #[test]
    fn test_extract_strict_reports_missing() {
        let v = json!({});
        assert!(extract_strict::<i64>(&v, "$.a.b").is_err());
        assert_eq!(extract_strict::<i64>(&json!({"a": {"b": 7}}), "$.a.b").unwrap(), 7);
    }
}
