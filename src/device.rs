//! 虚拟设备信息构造与持久化 (对应 Python 端 `utils/device.py`).

use rand::Rng;
use uuid::Uuid;

/// 生成满足 Luhn 校验的随机 IMEI.
pub fn random_imei() -> String {
    let mut rng = rand::thread_rng();
    let mut digits: Vec<u32> = (0..14).map(|_| rng.gen_range(0..10)).collect();
    let mut sum = 0;
    for (idx, d) in digits.iter().enumerate() {
        let mut checksum = *d;
        if idx % 2 == 1 {
            checksum *= 2;
            if checksum > 9 {
                checksum -= 9;
            }
        }
        sum += checksum;
    }
    let ctrl = (10 - (sum % 10)) % 10;
    digits.push(ctrl);
    digits.iter().map(|d| d.to_string()).collect()
}

/// 系统版本信息.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OSVersion {
    #[serde(default = "default_incremental")]
    pub incremental: String,
    #[serde(default = "default_release")]
    pub release: String,
    #[serde(default = "default_codename")]
    pub codename: String,
    #[serde(default = "default_sdk")]
    pub sdk: i32,
}

fn default_incremental() -> String {
    "5891938".into()
}
fn default_release() -> String {
    "10".into()
}
fn default_codename() -> String {
    "REL".into()
}
fn default_sdk() -> i32 {
    29
}

impl Default for OSVersion {
    fn default() -> Self {
        OSVersion {
            incremental: default_incremental(),
            release: default_release(),
            codename: default_codename(),
            sdk: default_sdk(),
        }
    }
}

/// 设备相关信息.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Device {
    pub display: String,
    pub product: String,
    pub device: String,
    pub board: String,
    pub model: String,
    pub fingerprint: String,
    pub boot_id: String,
    pub proc_version: String,
    pub imei: String,
    pub brand: String,
    pub bootloader: String,
    pub base_band: String,
    #[serde(default)]
    pub version: OSVersion,
    pub sim_info: String,
    pub os_type: String,
    pub mac_address: String,
    pub wifi_bssid: String,
    pub wifi_ssid: String,
    pub imsi_md5: Vec<u8>,
    pub android_id: String,
    pub apn: String,
    pub vendor_name: String,
    pub vendor_os_name: String,
    pub qimei: Option<String>,
    pub qimei36: Option<String>,
    pub qimei_save_time: Option<i64>,
    pub session_uid: Option<String>,
    pub session_sid: Option<String>,
    pub session_vkey: Option<String>,
    pub session_save_time: Option<i64>,
    pub open_udid: String,
}

fn rand_hex(n: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..n).map(|_| format!("{:x}", rng.gen_range(0..16))).collect()
}

impl Device {
    /// 生成一台随机模拟设备.
    pub fn random() -> Self {
        let mut rng = rand::thread_rng();
        let imsi_seed: Vec<u8> = (0..16).map(|_| rng.gen_range(0..255)).collect();
        let imsi_md5 = md5_digest(&imsi_seed);

        Device {
            display: format!("QMAPI.{}.001", rng.gen_range(100000..999999)),
            product: "iarim".into(),
            device: "sagit".into(),
            board: "eomam".into(),
            model: "MI 6".into(),
            fingerprint: format!(
                "xiaomi/iarim/sagit:10/eomam.200122.001/{}:user/release-keys",
                rng.gen_range(1000000..9999999)
            ),
            boot_id: Uuid::new_v4().to_string(),
            proc_version: format!(
                "Linux 5.4.0-54-generic-{} (android-build@google.com)",
                rand_hex(8)
            ),
            imei: random_imei(),
            brand: "Xiaomi".into(),
            bootloader: "U-boot".into(),
            base_band: String::new(),
            version: OSVersion::default(),
            sim_info: "T-Mobile".into(),
            os_type: "android".into(),
            mac_address: "00:50:56:C0:00:08".into(),
            wifi_bssid: "00:50:56:C0:00:08".into(),
            wifi_ssid: "<unknown ssid>".into(),
            imsi_md5,
            android_id: rand_hex(8),
            apn: "wifi".into(),
            vendor_name: "MIUI".into(),
            vendor_os_name: "qmapi".into(),
            qimei: None,
            qimei36: None,
            qimei_save_time: None,
            session_uid: None,
            session_sid: None,
            session_vkey: None,
            session_save_time: None,
            open_udid: Uuid::new_v4().simple().to_string(),
        }
    }
}

fn md5_digest(data: &[u8]) -> Vec<u8> {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

impl Default for Device {
    fn default() -> Self {
        Device::random()
    }
}
