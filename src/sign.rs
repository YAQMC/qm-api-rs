//! QQ 音乐请求签名算法.
//!
//! 官方桌面客户端 (`musics.fcg`) 需要将 payload 进行签名后才可访问,
//! 客户端使用 `__TENCENT_CHAOS_VM` 混淆虚拟机生成 `sign`; 该 VM 的输入
//! 与输出与下面实现的 `zzc_sign` 等价, 但使用 SHA1 的简化实现即可通过校验.

use base64::Engine;
use sha1::{Digest, Sha1};

/// 从 SHA1 十六进制摘要中挑选字符的索引集合.
const PART_1_INDEXES: [usize; 7] = [23, 14, 6, 36, 16, 7, 19];
const PART_2_INDEXES: [usize; 8] = [16, 1, 32, 12, 19, 27, 8, 5];
/// 与摘要字节进行异或的混淆常量.
const SCRAMBLE_VALUES: [u8; 20] = [
    89, 39, 179, 150, 218, 82, 58, 252, 177, 52, 186, 123, 120, 64, 242, 133, 143, 161, 121, 179,
];

/// 计算 QQ 音乐客户端请求的 `zzc` 签名.
///
/// Args:
///     payload: 待签名的明文 (UTF-8 字节或字符串).
///
/// Returns:
///     形如 `zzc...` 的小写签名串.
pub fn zzc_sign(payload: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(payload);
    let hash_hex = hex::encode(hasher.finalize()).to_uppercase();

    let part1: String = PART_1_INDEXES
        .iter()
        .map(|&i| hash_hex[i..i + 1].to_string())
        .collect();
    let part2: String = PART_2_INDEXES
        .iter()
        .map(|&i| hash_hex[i..i + 1].to_string())
        .collect();

    let mut part3 = [0u8; 20];
    for (i, v) in SCRAMBLE_VALUES.iter().enumerate() {
        let byte = u8::from_str_radix(&hash_hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
        part3[i] = v ^ byte;
    }
    let b64 = base64::engine::general_purpose::STANDARD
        .encode(part3)
        .replace(['/', '\\', '+', '='], "");

    format!("zzc{part1}{b64}{part2}").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zzc_sign_known() {
        // 固定测试向量.
        let payload = br#"{"comm":{"ct":19,"cv":1,"tmeAppID":"qqmusic"}}"#;
        let sign = zzc_sign(payload);
        assert!(
            sign.starts_with("zzc"),
            "sign should start with zzc, got {sign}"
        );
        assert_eq!(sign.len(), 44);
        assert_eq!(sign, "zzc667ce25ux835nhknj1bysb9mzdj7witfce03a5004");
    }
}
