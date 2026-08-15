//! 通用工具函数 (对应 Python 端 `utils/common.py`).

use md5::{Digest, Md5};
use rand::Rng;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// 计算字符串或字节串的 MD5 十六进制摘要.
pub fn calc_md5(strings: &[&[u8]]) -> String {
    let mut hasher = Md5::new();
    for s in strings {
        hasher.update(s);
    }
    hex::encode(hasher.finalize())
}

/// 生成随机 GUID (32 位十六进制字符串).
pub fn get_guid() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Hash33 算法 (用于 g_tk 计算).
///
/// 对每个字符执行 `h = (h << 5) + h + ord(c)`, 最后取 `0x7fffffff & h`.
pub fn hash33(s: &str, h: i64) -> i64 {
    let mut h = h;
    for c in s.chars() {
        let ord = c as i64;
        h = ((h << 5) + h + ord) & 0x7fffffff;
    }
    h
}

/// 生成随机 searchID 字符串.
pub fn get_search_id() -> String {
    let mut rng = rand::thread_rng();
    let e: i64 = rng.gen_range(1..=20);
    let t = e * 18_014_398_509_481_984;
    let n = rng.gen_range(0..=4_194_304) * 4_294_967_296;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let r = now % (24 * 60 * 60 * 1000);
    (t + n + r).to_string()
}

/// 递归将 JSON 值中的布尔值转换为整数 (0/1).
pub fn bool_to_int(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Bool(b) => serde_json::Value::from(*b as i32),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(bool_to_int).collect())
        }
        serde_json::Value::Object(map) => {
            let new_map = map
                .iter()
                .map(|(k, v)| (k.clone(), bool_to_int(v)))
                .collect();
            serde_json::Value::Object(new_map)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash33() {
        assert_eq!(hash33("placeholder-musickey", 5381), 1143673215);
    }

    #[test]
    fn test_md5() {
        assert_eq!(
            calc_md5(&[b"hello", b"world"]),
            "fc5e038d38a57032085441e7fe7010b0"
        );
    }
}
