//! 虚拟设备信息构造与持久化 (对应 Python 端 `utils/device.py`).

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::sync::OnceLock;
use uuid::Uuid;

/// 生成满足 Luhn 校验的随机 IMEI.
pub fn random_imei() -> String {
    let mut rng = rand::thread_rng();
    random_imei_with(&mut rng)
}

fn random_imei_with<R: Rng + ?Sized>(rng: &mut R) -> String {
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

/// 内置且内部一致的 Android 设备档案.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceProfile {
    Vivo,
    Xiaomi,
    Oppo,
}

impl DeviceProfile {
    /// 根据不区分大小写的档案名构造设备档案.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "vivo" => Some(Self::Vivo),
            "xiaomi" => Some(Self::Xiaomi),
            "oppo" => Some(Self::Oppo),
            _ => None,
        }
    }

    /// 档案名.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vivo => "vivo",
            Self::Xiaomi => "xiaomi",
            Self::Oppo => "oppo",
        }
    }

    fn data(self) -> &'static DeviceProfileData {
        match self {
            Self::Vivo => &VIVO_PROFILE,
            Self::Xiaomi => &XIAOMI_PROFILE,
            Self::Oppo => &OPPO_PROFILE,
        }
    }
}

struct DeviceProfileData {
    display: &'static str,
    product: &'static str,
    device: &'static str,
    board: &'static str,
    model: &'static str,
    fingerprint: &'static str,
    proc_version: &'static str,
    brand: &'static str,
    manufacturer: &'static str,
    host: &'static str,
    first_api_level: i32,
    vendor_name: &'static str,
    vendor_os_name: &'static str,
}

static VIVO_PROFILE: DeviceProfileData = DeviceProfileData {
    display: "PD2408D_A_16.1.18.2.W10",
    product: "PD2408",
    device: "PD2408",
    board: "sun",
    model: "V2408A",
    fingerprint: "vivo/PD2408/PD2408:15/AP3A.240905.015.A2/compiler250423182036:user/release-keys",
    proc_version: "Linux localhost 6.6.89-android15-8-g1f71897ac249-abogki467805059-4k #1 SMP PREEMPT Thu Dec 11 01:56:00 UTC 2025 aarch64",
    brand: "vivo",
    manufacturer: "vivo",
    host: "comdg01150014",
    first_api_level: 35,
    vendor_name: "OriginOS",
    vendor_os_name: "OriginOS 5.0",
};

static XIAOMI_PROFILE: DeviceProfileData = DeviceProfileData {
    display: "OS3.0.260511.1.WOCCNXM.STABLE-OS31",
    product: "dada",
    device: "dada",
    board: "sun",
    model: "24129PN74C",
    fingerprint: "Xiaomi/dada/dada:16/BP2A.250605.031.A3/OS3.0.260511.1.WOCCNXM.STABLE-OS31:user/release-keys",
    proc_version: "",
    brand: "Xiaomi",
    manufacturer: "Xiaomi",
    host: "",
    first_api_level: 35,
    vendor_name: "Xiaomi",
    vendor_os_name: "HyperOS 3.0",
};

static OPPO_PROFILE: DeviceProfileData = DeviceProfileData {
    display: "V.1ab312a_1-2a261",
    product: "PKB110",
    device: "OP5A3DL1",
    board: "mt6991",
    model: "PKB110",
    fingerprint: "OPPO/PKB110/OP5A3DL1:15/AP3A.240617.008/V.1ab312a_1-2a261:user/release-keys",
    proc_version: "",
    brand: "OPPO",
    manufacturer: "OPPO",
    host: "",
    first_api_level: 35,
    vendor_name: "ColorOS",
    vendor_os_name: "ColorOS 15",
};

fn profile_version(profile: DeviceProfile) -> OSVersion {
    match profile {
        DeviceProfile::Vivo => OSVersion {
            incremental: "compiler250423182036".into(),
            release: "15".into(),
            codename: "REL".into(),
            sdk: 35,
        },
        DeviceProfile::Xiaomi => OSVersion {
            incremental: "OS3.0.260511.1.WOCCNXM.STABLE-OS31".into(),
            release: "16".into(),
            codename: "REL".into(),
            sdk: 36,
        },
        DeviceProfile::Oppo => OSVersion {
            incremental: "V.1ab312a_1-2a261".into(),
            release: "15".into(),
            codename: "REL".into(),
            sdk: 35,
        },
    }
}

/// 设备相关信息.
///
/// 字段与序列化形状保持向后兼容。旧版本持久化的设备缺少
/// `open_udid2` 等新字段时，会在反序列化阶段生成安全且独立的标识。
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
    pub open_udid: String,
    #[serde(default = "default_open_udid2")]
    pub open_udid2: String,
    #[serde(default = "default_manufacturer")]
    pub manufacturer: String,
    #[serde(default = "default_device_host")]
    pub host: String,
    #[serde(default = "default_first_api_level")]
    pub first_api_level: i32,
}

fn default_open_udid2() -> String {
    Uuid::new_v4().simple().to_string()
}
fn default_manufacturer() -> String {
    String::new()
}
fn default_device_host() -> String {
    String::new()
}
fn default_first_api_level() -> i32 {
    29
}

fn random_mac<R: Rng + ?Sized>(rng: &mut R) -> String {
    let octets = [
        0x02u8,
        rng.gen(),
        rng.gen(),
        rng.gen(),
        rng.gen(),
        rng.gen(),
    ];
    octets
        .iter()
        .map(|octet| format!("{octet:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn from_rng<R: Rng + ?Sized>(profile: DeviceProfile, rng: &mut R) -> Device {
    let data = profile.data();
    let mut imsi_seed = vec![0u8; 16];
    rng.fill(imsi_seed.as_mut_slice());
    let imsi_md5 = md5_digest(&imsi_seed);
    let mac_address = random_mac(rng);
    let wifi_bssid = random_mac(rng);

    Device {
        display: data.display.into(),
        product: data.product.into(),
        device: data.device.into(),
        board: data.board.into(),
        model: data.model.into(),
        fingerprint: data.fingerprint.into(),
        boot_id: Uuid::from_u128(rng.gen()).to_string(),
        proc_version: data.proc_version.into(),
        imei: random_imei_with(rng),
        brand: data.brand.into(),
        bootloader: "U-boot".into(),
        base_band: String::new(),
        version: profile_version(profile),
        sim_info: "T-Mobile".into(),
        os_type: "android".into(),
        mac_address,
        wifi_bssid,
        wifi_ssid: "<unknown ssid>".into(),
        imsi_md5,
        android_id: format!("{:016x}", rng.gen::<u64>()),
        apn: "wifi".into(),
        vendor_name: data.vendor_name.into(),
        vendor_os_name: data.vendor_os_name.into(),
        qimei: None,
        qimei36: None,
        qimei_save_time: None,
        open_udid: Uuid::from_u128(rng.gen()).simple().to_string(),
        open_udid2: Uuid::from_u128(rng.gen()).simple().to_string(),
        manufacturer: data.manufacturer.into(),
        host: data.host.into(),
        first_api_level: data.first_api_level,
    }
}

impl Device {
    /// 使用指定档案生成模拟设备；提供 `seed` 时结果可复现.
    pub fn with_profile(profile: DeviceProfile, seed: Option<u64>) -> Self {
        match seed {
            Some(seed) => {
                let mut rng = StdRng::seed_from_u64(seed);
                from_rng(profile, &mut rng)
            }
            None => {
                let mut rng = rand::thread_rng();
                from_rng(profile, &mut rng)
            }
        }
    }

    /// 生成一台随机模拟设备.
    pub fn random() -> Self {
        static PROFILES: OnceLock<[DeviceProfile; 3]> = OnceLock::new();
        let profiles = PROFILES.get_or_init(|| {
            [
                DeviceProfile::Vivo,
                DeviceProfile::Xiaomi,
                DeviceProfile::Oppo,
            ]
        });
        let mut rng = rand::thread_rng();
        let profile = profiles[rng.gen_range(0..profiles.len())];
        from_rng(profile, &mut rng)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_profiles_are_coherent_and_reproducible() {
        let first = Device::with_profile(DeviceProfile::Xiaomi, Some(42));
        let second = Device::with_profile(DeviceProfile::Xiaomi, Some(42));
        assert_eq!(first.model, "24129PN74C");
        assert_eq!(first.version.release, "16");
        assert_eq!(first.first_api_level, 35);
        assert_eq!(first.android_id, second.android_id);
        assert_eq!(first.open_udid, second.open_udid);
        assert_ne!(first.open_udid, first.open_udid2);
    }

    #[test]
    fn legacy_device_payload_generates_secondary_udid() {
        let value = serde_json::json!({
            "display":"legacy", "product":"legacy", "device":"legacy", "board":"legacy",
            "model":"legacy", "fingerprint":"legacy", "boot_id":"boot", "proc_version":"proc",
            "imei":"imei", "brand":"brand", "bootloader":"boot", "base_band":"",
            "sim_info":"", "os_type":"android", "mac_address":"", "wifi_bssid":"",
            "wifi_ssid":"", "imsi_md5":[], "android_id":"aid", "apn":"wifi",
            "vendor_name":"", "vendor_os_name":"", "open_udid":"primary"
        });
        let device: Device = serde_json::from_value(value).unwrap();
        assert_eq!(device.open_udid, "primary");
        assert!(!device.open_udid2.is_empty());
        assert_ne!(device.open_udid, device.open_udid2);
        assert!(device.manufacturer.is_empty());
        assert!(device.host.is_empty());
    }
}
