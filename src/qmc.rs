//! QMC 加密音频解密 (QMCv1 静态密钥 + QMCv2 Map/RC4 + EKey 双层 TEA + Footer 解析).
//!
//! 移植自 [mzj3920/qqmusic-decrypt](https://github.com/mzj3920/qqmusic-decrypt)
//! (纯 Python 零依赖实现), 该实现以 unlock-music 官方 Rust 实现
//! (`lib_um_crypto_rust`) 为参照逐一对拍移植。
//!
//! 仅用于解密你自己合法下载、有权使用的音频文件。

use base64::Engine;
use serde_json::{json, Value};

use crate::error::{QmError, Result};

// ---------------------------------------------------------------------------
// 通用常量
// ---------------------------------------------------------------------------

const V1_KEY_SIZE: usize = 128;
const V1_OFFSET_BOUNDARY: usize = 0x7FFF;

/// QMCv1 静态密钥.
const V1_STATIC_KEY: [u8; 128] = [
    0xc3, 0x4a, 0xd6, 0xca, 0x90, 0x67, 0xf7, 0x52, 0xd8, 0xa1, 0x66, 0x62, 0x9f, 0x5b, 0x09, 0x00,
    0xc3, 0x5e, 0x95, 0x23, 0x9f, 0x13, 0x11, 0x7e, 0xd8, 0x92, 0x3f, 0xbc, 0x90, 0xbb, 0x74, 0x0e,
    0xc3, 0x47, 0x74, 0x3d, 0x90, 0xaa, 0x3f, 0x51, 0xd8, 0xf4, 0x11, 0x84, 0x9f, 0xde, 0x95, 0x1d,
    0xc3, 0xc6, 0x09, 0xd5, 0x9f, 0xfa, 0x66, 0xf9, 0xd8, 0xf0, 0xf7, 0xa0, 0x90, 0xa1, 0xd6, 0xf3,
    0xc3, 0xf3, 0xd6, 0xa1, 0x90, 0xa0, 0xf7, 0xf0, 0xd8, 0xf9, 0x66, 0xfa, 0x9f, 0xd5, 0x09, 0xc6,
    0xc3, 0x1d, 0x95, 0xde, 0x9f, 0x84, 0x11, 0xf4, 0xd8, 0x51, 0x3f, 0xaa, 0x90, 0x3d, 0x74, 0x47,
    0xc3, 0x0e, 0x74, 0xbb, 0x90, 0xbc, 0x3f, 0x92, 0xd8, 0x7e, 0x11, 0x13, 0x9f, 0x23, 0x95, 0x5e,
    0xc3, 0x00, 0x09, 0x5b, 0x9f, 0x62, 0x66, 0xa1, 0xd8, 0x52, 0xf7, 0x67, 0x90, 0xca, 0xd6, 0x4a,
];

/// 对整段数据做 V1 变换 (绝对偏移语义, 与参考实现 `_transform` 一致).
fn transform(data: &[u8], offset_start: usize, key: &[u8; 128]) -> Vec<u8> {
    let mut out = data.to_vec();
    let mut pos = offset_start;
    let mut i = 0;
    let total = out.len();
    while i < total {
        let (take, phase) = if pos <= V1_OFFSET_BOUNDARY {
            (std::cmp::min(0x8000 - pos, total - i), pos % V1_KEY_SIZE)
        } else {
            let r = pos % V1_OFFSET_BOUNDARY;
            (std::cmp::min(V1_OFFSET_BOUNDARY - r, total - i), r % V1_KEY_SIZE)
        };
        let base: Vec<u8> = if phase == 0 {
            key.to_vec()
        } else {
            [&key[phase..], &key[..phase]].concat()
        };
        let mut idx = 0usize;
        for j in 0..take {
            out[i + j] ^= base[idx];
            idx = (idx + 1) % V1_KEY_SIZE;
        }
        i += take;
        pos += take;
    }
    out
}

/// 整文件静态密钥解密 (offset 从 0 开始).
pub fn v1_decrypt(data: &[u8]) -> Vec<u8> {
    transform(data, 0, &V1_STATIC_KEY)
}

// ---------------------------------------------------------------------------
// Tencent TEA (tc_tea)
// ---------------------------------------------------------------------------

const TEA_DELTA: u32 = 0x9E37_79B9;
const TEA_ROUNDS: u32 = 16;
const TEA_SALT_LEN: usize = 2;
const TEA_ZERO_LEN: usize = 7;

fn tea_single_round(value: u32, s: u32, key1: u32, key2: u32) -> u32 {
    let left = value.wrapping_shl(4).wrapping_add(key1);
    let right = (value >> 5).wrapping_add(key2);
    let mid = s.wrapping_add(value);
    left ^ mid ^ right
}

fn tea_ecb_decrypt(block: u64, k: &[u32; 4]) -> u64 {
    let mut y = (block >> 32) as u32;
    let mut z = block as u32;
    let mut s = TEA_DELTA.wrapping_mul(TEA_ROUNDS);
    for _ in 0..16 {
        z = z.wrapping_sub(tea_single_round(y, s, k[2], k[3]));
        y = y.wrapping_sub(tea_single_round(z, s, k[0], k[1]));
        s = s.wrapping_sub(TEA_DELTA);
    }
    ((y as u64) << 32) | (z as u64)
}

#[cfg(test)]
fn tea_ecb_encrypt(block: u64, k: &[u32; 4]) -> u64 {
    let mut y = (block >> 32) as u32;
    let mut z = block as u32;
    let mut s: u32 = 0;
    for _ in 0..16 {
        s = s.wrapping_add(TEA_DELTA);
        y = y.wrapping_add(tea_single_round(z, s, k[0], k[1]));
        z = z.wrapping_add(tea_single_round(y, s, k[2], k[3]));
    }
    ((y as u64) << 32) | (z as u64)
}

fn tea_key_from16(key16: &[u8]) -> [u32; 4] {
    [
        u32::from_be_bytes([key16[0], key16[1], key16[2], key16[3]]),
        u32::from_be_bytes([key16[4], key16[5], key16[6], key16[7]]),
        u32::from_be_bytes([key16[8], key16[9], key16[10], key16[11]]),
        u32::from_be_bytes([key16[12], key16[13], key16[14], key16[15]]),
    ]
}

/// tc_tea "tweaked CBC" 解密. 返回去除 padding 后的明文.
fn tea_cbc_decrypt(ciphertext: &[u8], key16: &[u8]) -> Result<Vec<u8>> {
    let k = tea_key_from16(key16);
    if ciphertext.len() % 8 != 0 || ciphertext.len() < 10 {
        return Err(QmError::ApiData(format!("TEA: invalid cipher length {}", ciphertext.len())));
    }
    let mut iv1: u64 = 0;
    let mut iv2: u64 = 0;
    let mut out = Vec::with_capacity(ciphertext.len());
    for chunk in ciphertext.chunks(8) {
        let block = u64::from_be_bytes(chunk.try_into().unwrap());
        let result = block ^ iv2;
        let next_iv2 = tea_ecb_decrypt(result, &k);
        let p = next_iv2 ^ iv1;
        out.extend_from_slice(&p.to_be_bytes());
        iv1 = block;
        iv2 = next_iv2;
    }
    let pad_size = (out[0] & 0b111) as usize;
    let start = 1 + pad_size + TEA_SALT_LEN;
    let end = ciphertext.len() - TEA_ZERO_LEN;
    if out[end..].iter().any(|&b| b != 0) {
        return Err(QmError::ApiData("TEA: invalid padding".into()));
    }
    Ok(out[start..end].to_vec())
}

/// tc_tea 加密 (构造测试 EKey 用). salt 为 10 字节.
#[cfg(test)]
fn tea_cbc_encrypt(plaintext: &[u8], key16: &[u8], salt: &[u8]) -> Result<Vec<u8>> {
    let k = tea_key_from16(key16);
    let out_len_base = 10 + plaintext.len();
    let pad_len = (8 - (out_len_base & 7)) & 7;
    let header_len = 1 + pad_len + TEA_SALT_LEN;
    let out_len = out_len_base + pad_len;

    let mut header = [0u8; 16];
    header[..header_len].copy_from_slice(&salt[..header_len]);
    header[0] = (header[0] & !7) | pad_len as u8;
    let copy_len = std::cmp::min(16 - header_len, plaintext.len());
    header[header_len..header_len + copy_len].copy_from_slice(&plaintext[..copy_len]);
    let rest = &plaintext[copy_len..];

    let mut iv1: u64 = 0;
    let mut iv2: u64 = 0;
    let mut round_enc = |block: &[u8; 8]| -> [u8; 8] {
        let b = u64::from_be_bytes(*block);
        let iv2_next = b ^ iv1;
        let c = tea_ecb_encrypt(iv2_next, &k) ^ iv2;
        iv1 = c;
        iv2 = iv2_next;
        c.to_be_bytes()
    };

    let mut out = vec![0u8; out_len];
    out[0..8].copy_from_slice(&round_enc(header[0..8].try_into().unwrap()));
    out[8..16].copy_from_slice(&round_enc(header[8..16].try_into().unwrap()));
    let mut pos = 16;
    let mut rest = rest;
    while rest.len() >= 8 {
        out[pos..pos + 8].copy_from_slice(&round_enc(rest[..8].try_into().unwrap()));
        rest = &rest[8..];
        pos += 8;
    }
    if !rest.is_empty() {
        let mut padded = [0u8; 8];
        padded[..rest.len()].copy_from_slice(rest);
        out[pos..pos + 8].copy_from_slice(&round_enc(&padded));
    }
    out.truncate(out_len);
    Ok(out)
}

// ---------------------------------------------------------------------------
// EKey 解密
// ---------------------------------------------------------------------------

/// base64("QQMusic EncV2,Key:")
const EKEY_V2_PREFIX: &str = "UVFNdXNpYyBFbmNWMixLZXk6";
const EKEY_V2_KEY1: [u8; 16] = [
    0x33, 0x38, 0x36, 0x5A, 0x4A, 0x59, 0x21, 0x40, 0x23, 0x2A, 0x24, 0x25, 0x5E, 0x26, 0x29, 0x28,
];
const EKEY_V2_KEY2: [u8; 16] = [
    0x2A, 0x2A, 0x23, 0x21, 0x28, 0x23, 0x24, 0x25, 0x26, 0x5E, 0x61, 0x31, 0x63, 0x5A, 0x2C, 0x54,
];

/// 生成 V1 EKey 的简单密钥 (f32 语义, 饱和转换).
fn make_simple_key() -> [u8; 8] {
    let f01 = 0.1f32;
    let mut result = [0u8; 8];
    for i in 0..8 {
        let v = 106.0f32 + (i as f32) * f01;
        let t = v.tan();
        let v2 = t * 100.0f32;
        result[i] = v2 as u8;
    }
    result
}

fn simple_key() -> &'static [u8; 8] {
    use std::sync::OnceLock;
    static KEY: OnceLock<[u8; 8]> = OnceLock::new();
    KEY.get_or_init(make_simple_key)
}

/// V1 EKey: base64 -> header(8) + cipher, TEA 密钥 = 交错(simple_key, header).
fn ekey_decrypt_v1(ekey_bytes: &[u8]) -> Result<Vec<u8>> {
    if ekey_bytes.len() < 12 {
        return Err(QmError::ApiData("EKey 太短, 无法解密".into()));
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(ekey_bytes)
        .map_err(|e| QmError::ApiData(format!("EKey base64 解码失败: {e}")))?;
    if decoded.len() < 8 {
        return Err(QmError::ApiData("EKey base64 解码后不足 8 字节".into()));
    }
    let (header, cipher) = decoded.split_at(8);
    let mut tea_key = Vec::with_capacity(16);
    for (sk, hk) in simple_key().iter().zip(header) {
        tea_key.push(*sk);
        tea_key.push(*hk);
    }
    let plain = tea_cbc_decrypt(cipher, &tea_key)?;
    let mut out = Vec::with_capacity(8 + plain.len());
    out.extend_from_slice(header);
    out.extend_from_slice(&plain);
    Ok(out)
}

/// V2 EKey: 双层 TEA(KEY1, KEY2) 解密后去 0, 再走 V1.
fn ekey_decrypt_v2(ekey: &[u8]) -> Result<Vec<u8>> {
    let payload = base64::engine::general_purpose::STANDARD
        .decode(ekey)
        .map_err(|e| QmError::ApiData(format!("EKey V2 base64 解码失败: {e}")))?;
    let payload = tea_cbc_decrypt(&payload, &EKEY_V2_KEY1)?;
    let payload = tea_cbc_decrypt(&payload, &EKEY_V2_KEY2)?;
    let zero = payload.iter().position(|&b| b == 0).unwrap_or(payload.len());
    ekey_decrypt_v1(&payload[..zero])
}

/// 解密 EKey 得到主密钥 (master key). ekey 为文件中读出的字符串.
pub fn ekey_decrypt(ekey: &str) -> Result<Vec<u8>> {
    let bytes = ekey.as_bytes();
    if bytes.starts_with(EKEY_V2_PREFIX.as_bytes()) {
        return ekey_decrypt_v2(&bytes[EKEY_V2_PREFIX.len()..]);
    }
    ekey_decrypt_v1(bytes)
}

// ---------------------------------------------------------------------------
// QMC V2 流密码
// ---------------------------------------------------------------------------

/// 128 字节长密钥压缩 (对应 Rust v2_map/key.rs 的 key_compress).
fn key_compress(long_key: &[u8]) -> Vec<u8> {
    let n = long_key.len();
    let mut result = Vec::with_capacity(V1_KEY_SIZE);
    for i in 0..V1_KEY_SIZE {
        let idx = (i * i + 71214) % n;
        let key = long_key[idx];
        let shift = (idx + 4) % 8;
        result.push(((key << shift) | (key >> shift)) & 0xFF);
    }
    result
}

/// 短密钥 (1..300 字节) 的 Map 密码: 压缩后按 V1 变换异或.
pub struct Qmc2Map {
    key: Vec<u8>,
}

impl Qmc2Map {
    pub fn new(key: &[u8]) -> Result<Self> {
        if key.is_empty() {
            return Err(QmError::ApiData("Qmc2Map 密钥不能为空".into()));
        }
        Ok(Qmc2Map {
            key: key_compress(key),
        })
    }

    pub fn decrypt(&self, data: &[u8], offset: usize) -> Vec<u8> {
        let k: [u8; 128] = self.key.as_slice().try_into().expect("128 bytes");
        transform(data, offset, &k)
    }
}

/// 对应 Rust v2_rc4/hash.rs 的 hash().
fn qmc2_hash(key: &[u8]) -> f64 {
    let mut h: u32 = 1;
    for &v in key {
        if v == 0 {
            continue;
        }
        let nxt = h.wrapping_mul(v as u32);
        if nxt == 0 || nxt <= h {
            break;
        }
        h = nxt;
    }
    h as f64
}

/// 对应 Rust v2_rc4/segment_key.rs 的 get_segment_key.
fn get_segment_key(id: u64, seed: u64, h: f64) -> u64 {
    if seed == 0 {
        return 0;
    }
    let denom = (id + 1).wrapping_mul(seed);
    (h / denom as f64 * 100.0) as u64
}

/// Modified RC4: 状态长度 = 密钥长度, 状态为 u8 (i as u8 会模 256).
struct Rc4 {
    state: Vec<u8>,
    i: usize,
    j: usize,
    n: usize,
}

impl Rc4 {
    fn new(key: &[u8]) -> Self {
        let n = key.len();
        let mut state: Vec<u8> = (0..n).map(|i| (i & 0xFF) as u8).collect();
        let mut j = 0usize;
        for i in 0..n {
            j = (j + state[i] as usize + key[i % n] as usize) % n;
            state.swap(i, j);
        }
        Rc4 { state, i: 0, j: 0, n }
    }

    fn generate(&mut self) -> u8 {
        let n = self.n;
        self.i = (self.i + 1) % n;
        self.j = (self.j + self.state[self.i] as usize) % n;
        self.state.swap(self.i, self.j);
        let idx = (self.state[self.i] as usize + self.state[self.j] as usize) % n;
        self.state[idx]
    }
}

const RC4_FIRST_SEGMENT_SIZE: usize = 0x0080;
const RC4_OTHER_SEGMENT_SIZE: usize = 0x1400;
const RC4_STREAM_CACHE_SIZE: usize = RC4_OTHER_SEGMENT_SIZE + 512;

/// 长密钥 (>300 字节) 的 RC4 密码.
pub struct Qmc2Rc4 {
    hash: f64,
    key: Vec<u8>,
    key_stream: Vec<u8>,
}

impl Qmc2Rc4 {
    pub fn new(key: &[u8]) -> Result<Self> {
        if key.is_empty() {
            return Err(QmError::ApiData("Qmc2Rc4 密钥不能为空".into()));
        }
        let mut rc4 = Rc4::new(key);
        let hash = qmc2_hash(key);
        let key_stream: Vec<u8> = (0..RC4_STREAM_CACHE_SIZE).map(|_| rc4.generate()).collect();
        Ok(Qmc2Rc4 {
            hash,
            key: key.to_vec(),
            key_stream,
        })
    }

    fn process_first_segment(&self, buf: &mut [u8], start: usize, length: usize, offset: usize) {
        let n = self.key.len();
        for j in 0..length {
            let o = offset + j;
            let idx = get_segment_key(o as u64, self.key[o % n] as u64, self.hash) as usize % n;
            buf[start + j] ^= self.key[idx];
        }
    }

    fn process_other_segment(&self, buf: &mut [u8], start: usize, length: usize, offset: usize) {
        let n = self.key.len();
        let seg_id = offset / RC4_OTHER_SEGMENT_SIZE;
        let block_off = offset % RC4_OTHER_SEGMENT_SIZE;
        let seed = self.key[seg_id % n];
        let skip = get_segment_key(seg_id as u64, seed as u64, self.hash) as usize & 0x1FF;
        let base = skip + block_off;
        for j in 0..length {
            buf[start + j] ^= self.key_stream[base + j];
        }
    }

    pub fn decrypt(&self, data: &[u8], offset: usize) -> Vec<u8> {
        let mut out = data.to_vec();
        let n = out.len();
        let mut pos = offset;
        let mut start = 0usize;
        if pos < RC4_FIRST_SEGMENT_SIZE {
            let take = std::cmp::min(RC4_FIRST_SEGMENT_SIZE - pos, n - start);
            self.process_first_segment(&mut out, start, take, pos);
            start += take;
            pos += take;
        }
        if pos % RC4_OTHER_SEGMENT_SIZE != 0 {
            let take = std::cmp::min(RC4_OTHER_SEGMENT_SIZE - (pos % RC4_OTHER_SEGMENT_SIZE), n - start);
            self.process_other_segment(&mut out, start, take, pos);
            start += take;
            pos += take;
        }
        while start < n {
            let take = std::cmp::min(RC4_OTHER_SEGMENT_SIZE, n - start);
            self.process_other_segment(&mut out, start, take, pos);
            start += take;
            pos += take;
        }
        out
    }
}

/// 按主密钥长度选择密码.
enum Qmc2Cipher {
    Map(Qmc2Map),
    Rc4(Qmc2Rc4),
}

impl Qmc2Cipher {
    fn decrypt(&self, data: &[u8], offset: usize) -> Vec<u8> {
        match self {
            Qmc2Cipher::Map(m) => m.decrypt(data, offset),
            Qmc2Cipher::Rc4(r) => r.decrypt(data, offset),
        }
    }
}

fn make_qmc2_cipher(master_key: &[u8]) -> Result<Qmc2Cipher> {
    if master_key.is_empty() {
        return Err(QmError::ApiData("主密钥为空".into()));
    }
    if (1..=300).contains(&master_key.len()) {
        Ok(Qmc2Cipher::Map(Qmc2Map::new(master_key)?))
    } else {
        Ok(Qmc2Cipher::Rc4(Qmc2Rc4::new(master_key)?))
    }
}

// ---------------------------------------------------------------------------
// Footer 解析
// ---------------------------------------------------------------------------

/// 文件末尾元数据.
#[derive(Debug, Clone)]
pub struct FooterMetadata {
    /// 应从文件末尾裁剪的字节数.
    pub size: usize,
    /// 内嵌 EKey 字符串 (可能为 None).
    pub ekey: Option<String>,
    /// STag / QTag / PcV2MusicEx / PcV1Legacy.
    pub ftype: &'static str,
    /// media_mid / mid / media_filename 等附加信息.
    pub extra: Value,
}

fn is_base64_text(s: &[u8]) -> bool {
    s.iter().all(|&c| {
        c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'='
    })
}

fn read_utf16le(data: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i + 1 < data.len() {
        let lo = data[i];
        let hi = data[i + 1];
        if lo == 0 && hi == 0 {
            break;
        }
        if hi == 0 && lo > 0 && lo < 128 {
            out.push(lo as char);
        } else {
            break;
        }
        i += 2;
    }
    out
}

/// 解析文件末尾片段 (建议取最后 1024 字节).
///
/// 按参考实现顺序: STag -> QTag -> PcV2MusicEx -> PcV1Legacy.
pub fn parse_footer(tail: &[u8]) -> Result<Option<FooterMetadata>> {
    if tail.len() < 8 {
        return Ok(None);
    }

    // ---- Android STag (无 EKey) ----
    if tail.ends_with(b"STag") {
        let footer = &tail[..tail.len() - 4];
        let payload = &footer[..footer.len() - 4];
        let size_bytes: [u8; 4] = footer[footer.len() - 4..].try_into().unwrap();
        let payload_len = u32::from_be_bytes(size_bytes) as usize;
        if payload.len() < payload_len {
            return Err(QmError::ApiData("STag 长度不一致".into()));
        }
        let csv = String::from_utf8_lossy(&payload[payload.len() - payload_len..]).to_string();
        let parts: Vec<&str> = csv.split(',').collect();
        if parts.len() != 3 {
            return Err(QmError::ApiData("STag CSV 格式错误".into()));
        }
        let (rid, ver, media_mid) = (parts[0], parts[1], parts[2]);
        if ver != "2" {
            return Err(QmError::ApiData(format!("STag 版本不支持: {ver}")));
        }
        if !rid.chars().all(|c| c.is_ascii_digit()) {
            return Err(QmError::ApiData("STag ID 非法".into()));
        }
        return Ok(Some(FooterMetadata {
            size: payload_len + 8,
            ekey: None,
            ftype: "STag",
            extra: json!({ "resource_id": rid.parse::<i64>().unwrap_or(0), "media_mid": media_mid }),
        }));
    }

    // ---- Android QTag (含 EKey) ----
    if tail.ends_with(b"QTag") {
        let footer = &tail[..tail.len() - 4];
        let payload = &footer[..footer.len() - 4];
        let size_bytes: [u8; 4] = footer[footer.len() - 4..].try_into().unwrap();
        let payload_len = u32::from_be_bytes(size_bytes) as usize;
        if payload.len() < payload_len {
            return Err(QmError::ApiData("QTag 长度不一致".into()));
        }
        let csv = String::from_utf8_lossy(&payload[payload.len() - payload_len..]).to_string();
        let parts: Vec<&str> = csv.split(',').collect();
        if parts.len() != 3 {
            return Err(QmError::ApiData("QTag CSV 格式错误".into()));
        }
        let (ekey, rid, ver) = (parts[0], parts[1], parts[2]);
        if ver != "2" {
            return Err(QmError::ApiData(format!("QTag 版本不支持: {ver}")));
        }
        if !rid.chars().all(|c| c.is_ascii_digit()) {
            return Err(QmError::ApiData("QTag ID 非法".into()));
        }
        if !is_base64_text(ekey.as_bytes()) {
            return Err(QmError::ApiData("QTag EKey 非法".into()));
        }
        return Ok(Some(FooterMetadata {
            size: payload_len + 8,
            ekey: Some(ekey.to_string()),
            ftype: "QTag",
            extra: json!({ "resource_id": rid.parse::<i64>().unwrap_or(0) }),
        }));
    }

    // ---- PC MusicEx (无 EKey) ----
    if tail.ends_with(b"musicex\x00") {
        let payload = &tail[..tail.len() - 8];
        if payload.len() < 4 {
            return Err(QmError::ApiData("MusicEx 过短".into()));
        }
        let data = &payload[..payload.len() - 4];
        let version_bytes: [u8; 4] = payload[payload.len() - 4..].try_into().unwrap();
        let version = u32::from_le_bytes(version_bytes);
        if version != 1 {
            return Err(QmError::ApiData(format!("MusicEx 版本不支持: {version}")));
        }
        if data.len() < 4 {
            return Err(QmError::ApiData("MusicEx 过短".into()));
        }
        let payload2 = &data[..data.len() - 4];
        let len_bytes: [u8; 4] = data[data.len() - 4..].try_into().unwrap();
        let payload_len = u32::from_le_bytes(len_bytes) as usize;
        if payload_len != 0xC0 {
            return Err(QmError::ApiData(format!("MusicEx 长度非法: 0x{payload_len:X}")));
        }
        // 防恶意/损坏 footer: 校验内部载荷足够长, 避免 slice 越界 panic.
        let inner_len = payload2.len().saturating_sub(payload_len - 0x10);
        if inner_len < 12 + 60 + 100 {
            return Err(QmError::ApiData("MusicEx 内部载荷过短".into()));
        }
        let inner = &payload2[payload2.len() - inner_len..];
        let mid = read_utf16le(&inner[12..12 + 60]);
        let media_filename = read_utf16le(&inner[12 + 60..12 + 60 + 100]);
        return Ok(Some(FooterMetadata {
            size: payload_len,
            ekey: None,
            ftype: "PcV2MusicEx",
            extra: json!({ "mid": mid, "media_filename": media_filename }),
        }));
    }

    // ---- PC V1 Legacy (含 EKey, 经典 .mflac/.mgg) ----
    let payload = &tail[..tail.len() - 4];
    let size_bytes: [u8; 4] = tail[tail.len() - 4..].try_into().unwrap();
    let payload_len = u32::from_le_bytes(size_bytes) as usize;
    if payload_len > 0x500 {
        return Ok(None); // 可能是非 QMC 文件
    }
    if payload.len() < payload_len {
        return Err(QmError::ApiData("PCv1 长度不一致".into()));
    }
    let ekey_bytes = &payload[payload.len() - payload_len..];
    let zero = ekey_bytes.iter().position(|&b| b == 0).unwrap_or(ekey_bytes.len());
    let ekey_bytes = &ekey_bytes[..zero];
    if !is_base64_text(ekey_bytes) {
        return Err(QmError::ApiData("PCv1 EKey 非法".into()));
    }
    let ekey = String::from_utf8_lossy(ekey_bytes).to_string();
    Ok(Some(FooterMetadata {
        size: payload_len + 4,
        ekey: Some(ekey),
        ftype: "PcV1Legacy",
        extra: Value::Null,
    }))
}

// ---------------------------------------------------------------------------
// 音频类型嗅探
// ---------------------------------------------------------------------------

fn syncsafe_int(b4: &[u8]) -> usize {
    if b4.iter().any(|&c| c & 0x80 != 0) {
        return 0;
    }
    ((b4[0] as usize) << 21) | ((b4[1] as usize) << 14) | ((b4[2] as usize) << 7) | (b4[3] as usize)
}

/// 计算 ID3/APE 标签头长度; 无标签返回 0.
fn tag_header_size(buf: &[u8], offset: usize) -> usize {
    if buf.len() < offset + 10 {
        return 0;
    }
    let b = &buf[offset..];
    if b.starts_with(b"TAG") {
        return 128;
    }
    if b.starts_with(b"ID3") {
        return 10 + syncsafe_int(&b[6..10]);
    }
    if b.starts_with(b"APETAGEX") {
        if b.len() < 32 {
            return 0;
        }
        let extra = u32::from_le_bytes(b[0x0C..0x10].try_into().unwrap()) as usize;
        return 32 + extra;
    }
    0
}

/// 探测音频类型, 返回扩展名 ('bin' 表示未知).
pub fn detect_audio_type(data: &[u8]) -> String {
    let mut offset = 0usize;
    for _ in 0..5 {
        let ln = tag_header_size(data, offset);
        if ln == 0 {
            break;
        }
        offset += ln;
    }
    if data.len() < offset + 0x10 {
        return "bin".into();
    }
    let buf = &data[offset..];
    let magic4 = &buf[..4];
    let ext = match magic4 {
        b"fLaC" => "flac",
        b"OggS" => "ogg",
        b"FRM8" => "dff",
        b"RIFF" => "wav",
        b"MAC " => "ape",
        _ => "",
    };
    if !ext.is_empty() {
        return ext.into();
    }
    let magic = u32::from_be_bytes(magic4.try_into().unwrap());
    if (magic & 0xFFF6_0000) == 0xFFF0_0000 {
        return "aac".into();
    }
    if buf.len() >= 8 && &buf[4..8] == b"ftyp" {
        if buf.len() >= 12 {
            let major = &buf[8..12];
            if major == b"isom" || major == b"iso2" || major == b"MSNV" {
                return "mp4".into();
            }
            if major == b"NDAS" {
                return "m4a".into();
            }
            if buf.len() >= 11 {
                let major3 = &buf[8..11];
                if major3 == b"M4A" {
                    return "m4a".into();
                }
                if major3 == b"M4B" {
                    return "m4b".into();
                }
                if major3 == b"mp4" {
                    return "mp4".into();
                }
            }
        }
    }
    if data.len() >= 4096 {
        "bin".into()
    } else {
        // 数据不足时按现有头部判断
        "bin".into()
    }
}

/// 带重试的扩展名检测.
pub fn detect_audio_extension(data: &[u8]) -> String {
    let needed = 0x100.min(data.len());
    detect_audio_type(&data[..needed])
}

// ---------------------------------------------------------------------------
// 主流程
// ---------------------------------------------------------------------------

/// 解密单个 QMC 文件数据. 返回 (输出字节, 输出扩展名).
///
/// - 先尝试 v2 footer; 有内嵌 EKey 或提供了 `ekey_override` 则走 v2.
/// - 无 footer / 无法确定时回退 v1 静态密钥.
/// - 仍无 EKey 且无法离线解密的 (STag/MusicEx) 返回错误.
pub fn decrypt_qmc(data: &[u8], ekey_override: Option<&str>) -> Result<(Vec<u8>, String)> {
    let tail_start = data.len().saturating_sub(1024);
    let tail = &data[tail_start..];
    let footer = match parse_footer(tail) {
        Ok(f) => f,
        Err(_) => None,
    };

    // --- v2 路径 ---
    if let Some(footer) = &footer {
        let ekey = footer.ekey.clone().or_else(|| ekey_override.map(|s| s.to_string()));
        if let Some(ekey) = ekey {
            let master_key = ekey_decrypt(&ekey)?;
            let cipher = make_qmc2_cipher(&master_key)?;
            let audio = &data[..data.len() - footer.size];
            let out = cipher.decrypt(audio, 0);
            let ext = detect_audio_extension(&out);
            let ext = if ext == "bin" { "bin".into() } else { ext };
            return Ok((out, ext));
        }
        // footer 存在但无 EKey, 且未提供 override → 需要联网取 key
        return Err(QmError::ApiData(format!(
            "该文件未内嵌解密密钥 ({}), 需要在线获取密钥或提供 ekey",
            footer.ftype
        )));
    }

    // --- v1 静态密钥路径 ---
    let out = v1_decrypt(data);
    let ext = detect_audio_extension(&out);
    if ext == "bin" {
        return Err(QmError::ApiData("未能识别为受支持的 QMC 文件".into()));
    }
    Ok((out, ext))
}

/// 解密单个 QMC 文件并写入输出路径. 返回 (输出字节, 输出扩展名).
pub fn decrypt_file(input_path: &std::path::Path, ekey_override: Option<&str>) -> Result<(Vec<u8>, String)> {
    let data = std::fs::read(input_path).map_err(|e| QmError::Io(e.to_string()))?;
    let (out, ext) = decrypt_qmc(&data, ekey_override)?;
    Ok((out, ext))
}

/// 将解密后的音频写入磁盘, 自动选择扩展名.
pub fn decrypt_file_to(input_path: &std::path::Path, output_dir: &std::path::Path, ekey_override: Option<&str>) -> Result<std::path::PathBuf> {
    let (out, ext) = decrypt_file(input_path, ekey_override)?;
    std::fs::create_dir_all(output_dir)?;
    let stem = input_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".into());
    let ext = if ext == "bin" { "bin" } else { ext.as_str() };
    let out_path = output_dir.join(format!("{stem}.{ext}"));
    std::fs::write(&out_path, &out)?;
    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn gen_key() -> [u8; 128] {
        let mut k = [0u8; 128];
        for (i, v) in k.iter_mut().enumerate() {
            *v = (i + 1) as u8;
        }
        k
    }

    #[test]
    fn v1_transform_start() {
        let data = b"igohj&pg{fo";
        let out = transform(data, 0, &gen_key());
        assert_eq!(out, b"hello world");
    }

    #[test]
    fn v1_transform_boundary() {
        let d2 = [0x13, 0x19, 0x11, 0x12, 0x10, 0xa0, 0x75, 0x6c, 0x76, 0x69, 0x62];
        let out = transform(&d2, 0x7FFA, &gen_key());
        assert_eq!(out, b"hello world");
    }

    #[test]
    fn v1_whole_file() {
        let data = [0xab, 0x2f, 0xba, 0xa6, 0xff, 0x47, 0x80, 0x3d, 0xaa, 0xcd, 0x02];
        assert_eq!(v1_decrypt(&data), b"hello world");
    }

    #[test]
    fn key_compress_vector() {
        let test_key = (b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
            .repeat(6)
            .repeat(10))[..325]
            .to_vec();
        let expected = [
            0x79, 0xf4, 0x00, 0x75, 0x9e, 0x36, 0x00, 0x14, 0x8a, 0x63, 0x00, 0xb4, 0xbe, 0x77,
            0x00, 0x17, 0xba, 0x00, 0x37, 0x00, 0x00, 0x00, 0xbf, 0x80, 0x41, 0xbf, 0x83, 0xdd,
            0xbc, 0x5c, 0x02, 0x43, 0x14, 0x82, 0x49, 0x02, 0x00, 0x55, 0xbe, 0x6d, 0xbf, 0x49,
            0x80, 0x8e, 0x43, 0x00, 0xfa, 0x41, 0x67, 0xa8, 0x17, 0xf4, 0xae, 0x16, 0x15, 0x00,
            0xc1, 0x37, 0x82, 0xdd, 0x36, 0x21, 0x38, 0x55, 0x00, 0x79, 0x41, 0x9e, 0x42, 0xc1,
            0x36, 0xfa, 0xcf, 0x35, 0x00, 0x00, 0x41, 0xdd, 0x43, 0x42, 0x17, 0x4d, 0x8e, 0x8a,
            0xdd, 0x00, 0xbe, 0xf5, 0x38, 0xb4, 0xbf, 0x00, 0x7a, 0xcc, 0x4d, 0x02, 0x00, 0xcf,
            0xc1, 0xc1, 0x02, 0xa8, 0x00, 0x16, 0xc1, 0xbf, 0xc2, 0x42, 0x00, 0x49, 0x00, 0xc1,
            0xc2, 0xf5, 0x00, 0x17, 0x41, 0xdc, 0x83, 0xc2, 0x00, 0x9e, 0x41, 0xc1, 0x71, 0x36,
            0x00, 0x80,
        ];
        assert_eq!(key_compress(&test_key), expected);
    }

    #[test]
    fn map_decrypt_vector() {
        let test_key = (b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
            .repeat(6)
            .repeat(10))[..325]
            .to_vec();
        let cipher = Qmc2Map::new(&test_key).unwrap();
        let ct = [
            0x00, 0x9e, 0x41, 0xc1, 0x71, 0x36, 0x00, 0x80, 0xf4, 0x00, 0x75, 0x9e, 0x36, 0x00,
            0x14, 0x8a,
        ];
        assert_eq!(cipher.decrypt(&ct, 32760), vec![0u8; 16]);
    }

    #[test]
    fn hash_vector() {
        assert_eq!(qmc2_hash(b"hello world"), 4045008896.0);
    }

    #[test]
    fn segment_key_vector() {
        assert_eq!(get_segment_key(1, 0, 12345.0), 0);
        assert_eq!(get_segment_key(1, 123, 12345.0), 5018);
        assert_eq!(get_segment_key(51, 35, 516402887.0), 28373784);
        assert_eq!(get_segment_key(0, 66, 3908240000.0), 5921575757);
    }

    #[test]
    fn rc4_derive() {
        let mut rc4 = Rc4::new(b"this is a test key");
        let out: Vec<u8> = b"hello world".iter().map(|&v| v ^ rc4.generate()).collect();
        assert_eq!(
            out,
            [0x68, 0x75, 0x6b, 0x64, 0x64, 0x24, 0x7f, 0x60, 0x7c, 0x7d, 0x60]
        );
    }

    #[test]
    fn rc4_segment_decrypt() {
        let rc4_key = (b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
            .repeat(9)
            .repeat(10))[..512]
            .to_vec();
        // 参考 Python 自检的 256 字节密文 (末尾 0x1d).
        let ct: Vec<u8> = [
            0x39, 0x5a, 0x4f, 0x75, 0x38, 0x71, 0x37, 0x6b, 0x36, 0x51, 0x53, 0x6d, 0x7a, 0x66,
            0x53, 0x4b, 0x66, 0x50, 0x69, 0x34, 0x67, 0x6c, 0x33, 0x7a, 0x55, 0x62, 0x35, 0x5a,
            0x32, 0x75, 0x4f, 0x68, 0x44, 0x52, 0x6d, 0x65, 0x75, 0x6e, 0x39, 0x52, 0x30, 0x7a,
            0x68, 0x62, 0x73, 0x59, 0x39, 0x48, 0x55, 0x57, 0x73, 0x32, 0x5a, 0x70, 0x64, 0x50,
            0x4e, 0x52, 0x6a, 0x63, 0x4d, 0x39, 0x37, 0x76, 0x72, 0x47, 0x64, 0x4d, 0x62, 0x6d,
            0x58, 0x68, 0x75, 0x47, 0x37, 0x56, 0x69, 0x6b, 0x4a, 0x79, 0x66, 0x63, 0x70, 0x39,
            0x59, 0x34, 0x43, 0x6b, 0x45, 0x32, 0x5a, 0x31, 0x38, 0x77, 0x70, 0x43, 0x51, 0x79,
            0x6a, 0x62, 0x32, 0x33, 0x65, 0x58, 0x4a, 0x4d, 0x33, 0x4e, 0x70, 0x62, 0x62, 0x67,
            0x4c, 0x54, 0x78, 0x64, 0x64, 0x77, 0x6e, 0x72, 0x37, 0x41, 0x54, 0x39, 0x42, 0x52,
            0x47, 0x32, 0x1a, 0xe4, 0x1b, 0x71, 0x68, 0x29, 0xb3, 0x6e, 0xad, 0xc5, 0x28, 0x12,
            0xd6, 0xa4, 0x4b, 0x06, 0x7a, 0xdc, 0x90, 0x15, 0x99, 0xd6, 0xbf, 0x72, 0xa2, 0x30,
            0x37, 0x6b, 0x5c, 0xd6, 0x2f, 0x35, 0x14, 0x8a, 0xd6, 0xfb, 0x9f, 0xee, 0x7d, 0x2d,
            0xb7, 0x37, 0xf2, 0x0b, 0x6e, 0x00, 0xfb, 0xa0, 0x3c, 0x40, 0xf3, 0x36, 0xb2, 0x76,
            0x20, 0x0f, 0x9e, 0xa5, 0xa3, 0x15, 0x60, 0x23, 0x15, 0x29, 0xa1, 0x91, 0xbf, 0xfb,
            0x12, 0x95, 0xaa, 0x8d, 0x92, 0xc6, 0x0b, 0x8d, 0x49, 0x99, 0xa5, 0xe0, 0x05, 0xcf,
            0xb6, 0xac, 0x07, 0x54, 0x58, 0x28, 0xf9, 0x96, 0xd1, 0x9a, 0xfe, 0x0b, 0x3c, 0xfb,
            0x0b, 0x25, 0x7a, 0x43, 0x5a, 0x33, 0xc3, 0x7a, 0xfc, 0x33, 0xa3, 0xc2, 0x65, 0x48,
            0x29, 0x8d, 0x2c, 0x8f, 0x4e, 0x88, 0xfd, 0x44, 0xfd, 0xd5, 0xca, 0xb9, 0x8d, 0x62,
            0x4a, 0x48, 0x20, 0x1d,
        ]
        .to_vec();
        let cipher = Qmc2Rc4::new(&rc4_key).unwrap();
        assert_eq!(cipher.decrypt(&ct, 0), vec![0u8; 256]);
    }

    #[test]
    fn tea_decrypt_vector() {
        let good_ct = [
            0x91, 0x09, 0x51, 0x62, 0xe3, 0xf5, 0xb6, 0xdc, 0x6b, 0x41, 0x4b, 0x50, 0xd1, 0xa5,
            0xb8, 0x4e, 0xc5, 0x0d, 0x0c, 0x1b, 0x11, 0x96, 0xfd, 0x3c,
        ];
        let tea_key16 = *b"12345678ABCDEFGH";
        assert_eq!(
            tea_cbc_decrypt(&good_ct, &tea_key16).unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn tea_roundtrip() {
        let key = *b"43218765dcbahgfe";
        let salt = [0xA5, 0x6E, 0x35, 0xBC, 0x7C, 0x31, 0x04, 0x55, 0xA0, 0xBF];
        let enc = tea_cbc_encrypt(b"this is a test message.", &key, &salt).unwrap();
        assert_eq!(tea_cbc_decrypt(&enc, &key).unwrap(), b"this is a test message.");
    }

    #[test]
    fn ekey_v1_roundtrip() {
        // 构造 V1 EKey: header(8) + TEA 密文, 再解密回主密钥.
        let master_key: Vec<u8> = (0..16u8).collect();
        let header = &master_key[..8];
        let plain = &master_key[8..];
        let mut tea_key = Vec::new();
        for (sk, hk) in simple_key().iter().zip(header) {
            tea_key.push(*sk);
            tea_key.push(*hk);
        }
        let salt = [0xA5, 0x6E, 0x35, 0xBC, 0x7C, 0x31, 0x04, 0x55, 0xA0, 0xBF];
        let cipher = tea_cbc_encrypt(plain, &tea_key, &salt).unwrap();
        let mut encoded = header.to_vec();
        encoded.extend(&cipher);
        let ekey = base64::engine::general_purpose::STANDARD.encode(&encoded);
        assert_eq!(ekey_decrypt(&ekey).unwrap(), master_key);
    }

    #[test]
    fn ekey_v2_roundtrip() {
        // 构造 V2 EKey: 双层 TEA(KEY2, KEY1) 包裹 V1 EKey, 再解密回主密钥.
        let master_key: Vec<u8> = (0..16u8).collect();
        let header = &master_key[..8];
        let plain = &master_key[8..];
        let mut tea_key = Vec::new();
        for (sk, hk) in simple_key().iter().zip(header) {
            tea_key.push(*sk);
            tea_key.push(*hk);
        }
        let salt = [0xA5, 0x6E, 0x35, 0xBC, 0x7C, 0x31, 0x04, 0x55, 0xA0, 0xBF];
        let cipher = tea_cbc_encrypt(plain, &tea_key, &salt).unwrap();
        let mut encoded = header.to_vec();
        encoded.extend(&cipher);
        let v1_ekey = base64::engine::general_purpose::STANDARD.encode(&encoded);

        let mut inner = v1_ekey.clone().into_bytes();
        inner.extend_from_slice(&[0u8; 8]);
        let x = tea_cbc_encrypt(&inner, &EKEY_V2_KEY2, &salt).unwrap();
        let payload = tea_cbc_encrypt(&x, &EKEY_V2_KEY1, &salt).unwrap();
        let v2_ekey = format!(
            "{}{}",
            EKEY_V2_PREFIX,
            base64::engine::general_purpose::STANDARD.encode(&payload)
        );
        assert_eq!(ekey_decrypt(&v2_ekey).unwrap(), master_key);
    }

    #[test]
    fn footer_parse_qtag() {
        // 构造 QTag footer: CSV(ekey,rid,ver) + big-endian 长度 + "QTag".
        let ekey = "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo=";
        let csv = format!("{ekey},12345,2").into_bytes();
        let mut tail = csv.clone();
        tail.extend_from_slice(&(csv.len() as u32).to_be_bytes());
        tail.extend_from_slice(b"QTag");
        let meta = parse_footer(&tail).unwrap().unwrap();
        assert_eq!(meta.ftype, "QTag");
        assert_eq!(meta.ekey.as_deref(), Some(ekey));
        assert_eq!(meta.size, csv.len() + 8);
    }

    #[test]
    fn footer_parse_pc_v1() {
        let ekey = "UVFNdXNpYyBFbmNWMixLZXk6".as_bytes().to_vec();
        let mut tail = ekey.clone();
        tail.extend_from_slice(&(ekey.len() as u32).to_le_bytes());
        let meta = parse_footer(&tail).unwrap().unwrap();
        assert_eq!(meta.ftype, "PcV1Legacy");
        assert_eq!(meta.ekey.as_deref(), Some("UVFNdXNpYyBFbmNWMixLZXk6"));
    }

    #[test]
    fn sniff_audio() {
        let flac = [b"fLaC".to_vec(), vec![0u8; 100]].concat();
        assert_eq!(detect_audio_extension(&flac), "flac");
        let ogg = [b"OggS".to_vec(), vec![0u8; 100]].concat();
        assert_eq!(detect_audio_extension(&ogg), "ogg");
        // ID3v2 头 (size=11) + 11 字节帧数据, 音频从第 21 字节开始.
        let mut id3 = b"ID3\x04\x00\x00\x00\x00\x00\x0b".to_vec();
        id3.extend(vec![0u8; 11]);
        id3.extend(b"fLaC");
        id3.extend(vec![0u8; 100]);
        assert_eq!(detect_audio_extension(&id3), "flac");
        let garbage: Vec<u8> = (0u8..=255).cycle().take(2048).collect();
        assert_eq!(detect_audio_extension(&garbage), "bin");
    }

    #[test]
    fn empty_key_constructors_error_not_panic() {
        assert!(Qmc2Map::new(&[]).is_err());
        assert!(Qmc2Rc4::new(&[]).is_err());
        // 非空密钥正常.
        assert!(Qmc2Map::new(&[1, 2, 3]).is_ok());
        assert!(Qmc2Rc4::new(&[1, 2, 3]).is_ok());
    }

    #[test]
    fn musicex_short_inner_payload_errors_not_panic() {
        // 构造尾部 "musicex\0", 内部载荷远短于 0xC0, 不应 panic.
        let mut tail = vec![0u8; 4];
        tail.extend_from_slice(&0xC0u32.to_le_bytes());
        tail.extend_from_slice(b"musicex\x00");
        assert!(parse_footer(&tail).is_err());
    }

    #[test]
    fn malformed_footer_never_panics() {
        // 各类畸形尾部: 短尾部 / 随机字节, 只允许 Ok(None)/Err, 不允许 panic.
        for tail in [&b""[..], b"abc", b"QTag", b"\x00\x00\x00\x10QTag", b"musicex\x00", b"STag"] {
            let _ = parse_footer(tail);
        }
        let garbage: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
        for chunk in garbage.chunks(17) {
            let _ = parse_footer(chunk);
        }
    }
}
