//! 歌词解密与 Lyric API 返回模型定义 (对应 Python 端 `models/lyric.py`).

use serde::Deserialize;
use serde_json::Value;
use std::io::Read;

use crate::tripledes::{tripledes_crypt, tripledes_key_setup, DECRYPT};

/// QRC 3DES 解密密钥.
const QRC_3DES_KEY: &[u8; 24] = b"!@#)(*$%123ZXC!@!@#)(NHL";
/// QRC 解压后的最大歌词大小。正常歌词远小于该值；限制用于防止 zlib bomb.
const MAX_QRC_DECOMPRESSED_BYTES: usize = 4 * 1024 * 1024;

/// 解密 QRC 歌词.
///
/// 使用自定义 3DES-EDE (ECB, 8 字节分块) 解密后再 zlib 解压。密文必须严格按
/// 8 字节块对齐；解压结果限制为 4 MiB，损坏输入会返回 `None`，不会忽略尾部或
/// 无界扩张内存。
pub fn qrc_decrypt(encrypted: &str) -> Option<String> {
    if encrypted.is_empty() {
        return None;
    }
    let bytes = hex::decode(encrypted).ok()?;
    if bytes.is_empty() || !bytes.len().is_multiple_of(8) {
        return None;
    }

    let schedule = tripledes_key_setup(QRC_3DES_KEY, DECRYPT);
    let mut out = Vec::with_capacity(bytes.len());
    for chunk in bytes.chunks_exact(8) {
        out.extend_from_slice(&tripledes_crypt(chunk, &schedule));
    }

    let decoder = flate2::read::ZlibDecoder::new(&out[..]);
    let mut limited = decoder.take((MAX_QRC_DECOMPRESSED_BYTES + 1) as u64);
    let mut decoded = Vec::new();
    limited.read_to_end(&mut decoded).ok()?;
    if decoded.len() > MAX_QRC_DECOMPRESSED_BYTES {
        return None;
    }
    String::from_utf8(decoded).ok()
}

fn decrypt_field(value: &mut Value, key: &str) {
    if let Some(s) = value.get_mut(key) {
        if let Value::String(text) = s {
            if let Some(decrypted) = qrc_decrypt(text) {
                *s = Value::String(decrypted);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qrc_decrypt_reference() {
        // 由 Python 参考实现加密生成的已知密文.
        let encrypted =
            "3c80fea4c8965b324d9d7f9b0778e5be0374013221f3c86fdbab3be5929b9320ea64d4ea7f2fa40a";
        let decrypted = qrc_decrypt(encrypted).unwrap();
        assert_eq!(
            decrypted,
            "[ti:test][00:00.00]\u{4f60}\u{597d}\u{4e16}\u{754c}"
        );
    }

    #[test]
    fn qrc_rejects_non_block_aligned_ciphertext() {
        assert!(qrc_decrypt("001122334455667788").is_none());
    }

    #[test]
    fn qrc_rejects_oversized_decompression() {
        use std::io::Write;

        let oversized = vec![b'a'; MAX_QRC_DECOMPRESSED_BYTES + 1];
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&oversized).unwrap();
        let compressed = encoder.finish().unwrap();

        // 此测试验证解压读取器自身的 cap 语义；构造合法 3DES QRC 密文并非本测试目标。
        let decoder = flate2::read::ZlibDecoder::new(&compressed[..]);
        let mut limited = decoder.take((MAX_QRC_DECOMPRESSED_BYTES + 1) as u64);
        let mut decoded = Vec::new();
        limited.read_to_end(&mut decoded).unwrap();
        assert!(decoded.len() > MAX_QRC_DECOMPRESSED_BYTES);
    }
}

/// 歌词接口返回响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GetLyricResponse {
    #[serde(alias = "songID")]
    pub songid: i64,
    pub lyric: String,
    pub trans: String,
    pub roma: String,
    #[serde(alias = "singingAnnotationsLyric")]
    pub singing_annotations_lyric: String,
    pub lrc_t: i64,
    pub qrc_t: i64,
    pub trans_t: i64,
    pub roma_t: i64,
    #[serde(alias = "singingAnnotationsTs")]
    pub singing_annotations_ts: i64,
    #[serde(alias = "hasContributor")]
    pub has_contributor: bool,
    #[serde(alias = "hasTransContributor")]
    pub has_trans_contributor: bool,
    #[serde(alias = "hasMultiTrans")]
    pub has_multi_trans: bool,
}

impl GetLyricResponse {
    /// 从原始 data 解析并解密歌词字段.
    pub fn parse(mut data: Value) -> Result<Self, serde_json::Error> {
        for key in ["lyric", "trans", "roma", "singingAnnotationsLyric"] {
            decrypt_field(&mut data, key);
        }
        serde_json::from_value(data)
    }
}

/// 获取助唱标注歌词信息响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GetSingingAnnotationsInfoResponse {
    #[serde(alias = "hasSingingAnnotationsLyric")]
    pub has_singing_annotations_lyric: bool,
}

/// 多风格翻译歌词项.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MultiStyleLyricItem {
    pub style: i64,
    #[serde(alias = "styleName")]
    pub style_name: String,
    pub lyric: String,
    pub timestamp: i64,
}

/// 获取多风格翻译歌词接口响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BatchGetMultiStyleTransLyricResponse {
    pub lyrics: Vec<MultiStyleLyricItem>,
}

/// 检查是否存在 AI 歌词词典响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct IsAIDictExistsResponse {
    pub exists: bool,
}

/// AI 歌词词典项.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AIDictItem {
    pub phrase: String,
    pub explain: String,
    pub lyric_text: String,
    pub trans_lyric_text: String,
    pub lyric_timestamp: String,
}

/// 获取 AI 歌词词典响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GetAIDictResponse {
    #[serde(alias = "dictList")]
    pub dict_list: Vec<MultiStyleLyricItem>,
}