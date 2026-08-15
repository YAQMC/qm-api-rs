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
pub fn extract_typed<T: serde::de::DeserializeOwned + Default>(root: &Value, expr: &str) -> T {
    let value = extract(root, expr);
    if value.is_null() {
        return T::default();
    }
    serde_json::from_value(value).unwrap_or_default()
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
}
