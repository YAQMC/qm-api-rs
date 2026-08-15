//! QIMEI 获取与加密 (对应 Python 端 `utils/qimei.py`).
//!
//! Android 平台的 `comm` 参数需要携带 `QIMEI` / `QIMEI36` 指纹, 通过
//! `api.tencentmusic.com` 的 tRPC 代理申请. 请求负载使用 RSA (PKCS1v15)
//! 与 AES-128-CBC 加密.

use base64::Engine;
use rand::Rng;
use rsa::pkcs1v15::Pkcs1v15Encrypt;
use rsa::RsaPublicKey;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::utils::calc_md5;
use crate::Device;

const PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDEIxgwoutfwoJxcGQeedgP7FG9qaIuS0qzfR8gWkrkTZKM2iWHn2ajQpBRZjMSoSf6+KJGvar2ORhBfpDXyVtZCKpqLQ+FLkpncClKVIrBwv6PHyUvuCb0rIarmgDnzkfQAqVufEtR64iazGDKatvJ9y6B9NMbHddGSAUmRTCrHQIDAQAB
-----END PUBLIC KEY-----"#;
const SECRET: &str = "ZdJqM15EeO2zWc08";
const APP_KEY: &str = "0AND0HD6FE4HY80F";
const CHANNEL_ID: &str = "10003505";
const PACKAGE_ID: &str = "com.tencent.qqmusic";

/// RSA PKCS1v15 加密.
pub fn rsa_encrypt(content: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use rsa::pkcs8::DecodePublicKey;
    // `der` crate 的 PEM 解析对换行较严格, 这里手动剥壳后按 DER 解析.
    let body = PUBLIC_KEY
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<String>();
    let der = base64::engine::general_purpose::STANDARD.decode(body)?;
    let key = RsaPublicKey::from_public_key_der(&der)?;
    let mut rng = rand::thread_rng();
    Ok(key.encrypt(&mut rng, Pkcs1v15Encrypt, content)?)
}

/// AES-128-CBC 加密 (key 与 IV 相同, PKCS7 填充).
pub fn aes_encrypt(key: &[u8], content: &[u8]) -> Result<Vec<u8>, cbc::cipher::InvalidLength> {
    use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
    let cipher = cbc::Encryptor::<aes::Aes128>::new_from_slices(key, key)?;
    Ok(cipher.encrypt_padded_vec_mut::<Pkcs7>(content))
}

/// 将 Unix 秒转换为 UTC 的 (年, 月, 日, 时, 分, 秒).
fn utc_from_secs(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let h = sod / 3600;
    let mi = (sod % 3600) / 60;
    let s = sod % 60;
    (y, m, d, h as u32, mi as u32, s as u32)
}

/// Howard Hinnant 的 days -> (y, m, d) 算法.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d as u32)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// 随机生成灯塔 ID.
pub fn random_beacon_id() -> String {
    let mut rng = rand::thread_rng();
    let (y, m, _, _, _, _) = utc_from_secs(now_secs());
    let time_month = format!("{y}-{m:02}-01");
    let rand1: u64 = rng.gen_range(100000..999999);
    let rand2: u64 = rng.gen_range(100000000..999999999);

    let mut beacon_id = String::new();
    for i in 1..=40 {
        match i {
            1 | 2 | 13 | 14 | 17 | 18 | 21 | 22 | 25 | 26 | 29 | 30 | 33 | 34 | 37 | 38 => {
                beacon_id.push_str(&format!("k{i}:{time_month}{rand1}.{rand2}"));
            }
            3 => beacon_id.push_str("k3:0000000000000000"),
            4 => beacon_id.push_str(&format!("k4:{}", rand_hex_from(1..16, 16))),
            _ => beacon_id.push_str(&format!("k{i}:{}", rng.gen_range(0..9999))),
        }
        beacon_id.push(';');
    }
    beacon_id
}

fn rand_hex(n: usize) -> String {
    rand_hex_from(0..16, n)
}

fn rand_hex_from(range: std::ops::Range<u8>, n: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..n)
        .map(|_| format!("{:x}", rng.gen_range(range.clone())))
        .collect()
}

/// 根据设备信息随机生成 QIMEI 请求负载.
pub fn random_payload_by_device(device: &Device, version: &str, sdk_version: &str) -> Value {
    let mut rng = rand::thread_rng();
    let fixed_rand: i64 = rng.gen_range(0..14400);
    let (y, m, d, h, mi, s) = utc_from_secs(now_secs() - fixed_rand);
    let reserved = json!({
        "harmony": "0",
        "clone": "0",
        "containe": "",
        "oz": "UhYmelwouA+V2nPWbOvLTgN2/m8jwGB+yUB5v9tysQg=",
        "oo": "Xecjt+9S1+f8Pz2VLSxgpw==",
        "kelong": "0",
        "uptimes": format!("{y}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}"),
        "multiUser": "0",
        "bod": device.brand,
        "dv": device.device,
        "firstLevel": "",
        "manufact": device.brand,
        "name": device.model,
        "host": "se.infra",
        "kernel": device.proc_version,
    });
    json!({
        "androidId": device.android_id,
        "platformId": 1,
        "appKey": APP_KEY,
        "appVersion": version,
        "beaconIdSrc": random_beacon_id(),
        "brand": device.brand,
        "channelId": CHANNEL_ID,
        "cid": "",
        "imei": device.imei,
        "imsi": "",
        "mac": "",
        "model": device.model,
        "networkType": "unknown",
        "oaid": "",
        "osVersion": format!("Android {},level {}", device.version.release, device.version.sdk),
        "qimei": "",
        "qimei36": "",
        "sdkVersion": sdk_version,
        "targetSdkVersion": "33",
        "audit": "",
        "userId": "{}",
        "packageId": PACKAGE_ID,
        "deviceType": "Phone",
        "sdkName": "",
        "reserved": reserved.to_string(),
    })
}

/// 构建 QIMEI 请求头和请求体.
pub fn build_qimei_request(
    device: &Device,
    version: &str,
    sdk_version: &str,
) -> (i64, Vec<(String, String)>, Value) {
    let payload = random_payload_by_device(device, version, sdk_version);
    let crypt_key = rand_hex(16);
    let nonce = rand_hex(16);
    let ts = now_secs();

    let b64 = base64::engine::general_purpose::STANDARD;
    let key = b64.encode(rsa_encrypt(crypt_key.as_bytes()).expect("rsa encrypt"));
    let params = b64.encode(
        aes_encrypt(crypt_key.as_bytes(), payload.to_string().as_bytes()).expect("aes encrypt"),
    );
    let extra = format!("{{\"appKey\":\"{APP_KEY}\"}}");
    let req_sign = calc_md5(&[
        key.as_bytes(),
        params.as_bytes(),
        format!("{}", ts * 1000).as_bytes(),
        nonce.as_bytes(),
        SECRET.as_bytes(),
        extra.as_bytes(),
    ]);

    let header_sign = calc_md5(&[
        b"qimei_qq_androidpzAuCmaFAaFaHrdakPjLIEqKrGnSOOvH",
        format!("{ts}").as_bytes(),
    ]);

    let headers = vec![
        ("Host".to_string(), "api.tencentmusic.com".to_string()),
        ("method".to_string(), "GetQimei".to_string()),
        (
            "service".to_string(),
            "trpc.tme_datasvr.qimeiproxy.QimeiProxy".to_string(),
        ),
        ("appid".to_string(), "qimei_qq_android".to_string()),
        ("sign".to_string(), header_sign),
        ("user-agent".to_string(), "QQMusic".to_string()),
        ("timestamp".to_string(), ts.to_string()),
    ];

    let request_json = json!({
        "app": 0,
        "os": 1,
        "qimeiParams": {
            "key": key,
            "params": params,
            "time": ts.to_string(),
            "nonce": nonce,
            "sign": req_sign,
            "extra": extra,
        },
    });
    (ts, headers, request_json)
}

/// 解析 QIMEI 响应.
pub fn parse_qimei_response(body: &str) -> Option<(String, String)> {
    let outer: Value = serde_json::from_str(body).ok()?;
    let data_str = outer.get("data")?.as_str()?;
    let inner: Value = serde_json::from_str(data_str).ok()?;
    let data = inner.get("data")?;
    let q16 = data.get("q16")?.as_str()?;
    let q36 = data.get("q36")?.as_str()?;
    Some((q16.to_string(), q36.to_string()))
}
