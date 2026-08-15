//! 辅助功能工具: COS 文件上传会话 (对应 Python 端 `modules/helper_utils.py`).

use sha1::Digest;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use super::helper::HelperApi;
use crate::error::{QmError, Result};
use crate::models::helper::{
    FinishUploadResultDict, InitUploadFileDict, InitUploadResponse, UploadObjectInfo,
};
use crate::models::Credential;

/// 分块上传阈值 (5MB).
pub const MULTIPART_THRESHOLD: u64 = 5 * 1024 * 1024;
/// COS 上传重试次数.
pub const UPLOAD_RETRIES: u32 = 3;

/// 封装 COS 文件上传流程的会话对象.
pub struct UploadFileSession {
    pub api: HelperApi,
    pub bus_id: String,
    pub credential: Option<Credential>,
    pub max_concurrency: usize,
    init_data: Mutex<Option<InitUploadResponse>>,
    last_file_shas: Mutex<Option<Vec<String>>>,
}

impl std::fmt::Debug for UploadFileSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadFileSession")
            .field("bus_id", &self.bus_id)
            .field("max_concurrency", &self.max_concurrency)
            .finish()
    }
}

impl UploadFileSession {
    pub fn new(api: HelperApi, bus_id: &str) -> Self {
        UploadFileSession {
            api,
            bus_id: bus_id.to_string(),
            credential: None,
            max_concurrency: 3,
            init_data: Mutex::new(None),
            last_file_shas: Mutex::new(None),
        }
    }

    pub fn with_credential(mut self, credential: Credential) -> Self {
        self.credential = Some(credential);
        self
    }

    pub fn with_max_concurrency(mut self, max_concurrency: usize) -> Self {
        self.max_concurrency = max_concurrency;
        self
    }

    /// 计算文件 SHA1 摘要 (十六进制).
    pub fn sha1_file(path: &Path) -> Result<String> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)
            .map_err(|e| QmError::Io(format!("打开文件失败 {path:?}: {e}")))?;
        let mut hasher = sha1::Sha1::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = file
                .read(&mut buf)
                .map_err(|e| QmError::Io(format!("读取文件失败 {path:?}: {e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hex::encode(hasher.finalize()))
    }

    /// 获取单个文件的信息用于 InitUpload.
    pub fn get_file_info(path: &Path) -> Result<InitUploadFileDict> {
        if !path.is_file() {
            return Err(QmError::Io(format!("文件不存在: {path:?}")));
        }
        let metadata = std::fs::metadata(path)?;
        Ok(InitUploadFileDict {
            file_sha1: Self::sha1_file(path)?,
            file_name: path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            file_size: metadata.len() as i64,
        })
    }

    /// 准备上传, 获取或复用有效的临时凭证 (按文件 SHA1 去重).
    pub async fn prepare(&self, file_paths: &[PathBuf]) -> Result<()> {
        if file_paths.is_empty() {
            return Err(QmError::ValueError("至少需要提供一个文件路径".into()));
        }
        let file_infos: Result<Vec<InitUploadFileDict>> =
            file_paths.iter().map(|p| Self::get_file_info(p)).collect();
        let file_infos = file_infos?;
        let current_shas: Vec<String> = file_infos.iter().map(|f| f.file_sha1.clone()).collect();

        {
            let mut last = self.last_file_shas.lock().unwrap();
            if *last != Some(current_shas.clone()) {
                *last = Some(current_shas.clone());
                *self.init_data.lock().unwrap() = None;
            }
        }

        {
            let init = self.init_data.lock().unwrap();
            if let Some(data) = init.as_ref() {
                let now = now_secs();
                // 留 10 分钟余量防止临界过期
                if now < data.auth_info.expired_time - 600 {
                    return Ok(());
                }
            }
        }

        let init = self
            .api
            .init_upload(&self.bus_id, &file_infos, self.credential.as_ref())
            .await?;
        *self.init_data.lock().unwrap() = Some(init);
        Ok(())
    }

    /// 执行多文件的完整上传流程.
    ///
    /// 将文件直传到 COS, 然后调用 `FinishUpload` 通知服务器验证.
    pub async fn upload(&self, file_paths: &[PathBuf]) -> Result<Vec<UploadObjectInfo>> {
        if file_paths.is_empty() {
            return Err(QmError::ValueError("至少需要提供一个文件路径".into()));
        }
        self.prepare(file_paths).await?;
        let init = self
            .init_data
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| QmError::ApiData("获取上传凭证失败: 服务器未返回凭证信息".into()))?;

        let auth = &init.auth_info;
        if init.files.len() != file_paths.len() {
            return Err(QmError::ValueError(
                "InitUpload 返回的文件目标数量不匹配".into(),
            ));
        }
        if auth.secret_id.is_empty() || auth.secret_key.is_empty() || auth.token.is_empty() {
            return Err(QmError::ApiData("获取上传凭证失败: 凭证信息不完整".into()));
        }

        let mut finish_results = Vec::with_capacity(file_paths.len());
        for (i, file_info) in init.files.iter().enumerate() {
            let buckets = &file_info.buckets;
            let target = buckets.first().ok_or_else(|| {
                QmError::ApiData(format!(
                    "文件 {} 未返回目标存储桶信息",
                    file_paths[i].display()
                ))
            })?;
            let bucket_name = &target.bucket.name;
            let region = &target.bucket.region;
            let object_key = &file_info.object_key;

            if bucket_name.is_empty() || region.is_empty() || object_key.is_empty() {
                return Err(QmError::ApiData(format!(
                    "文件 {} 上传凭证信息不完整",
                    file_paths[i].display()
                )));
            }

            if target.upload_status != 1 {
                self.api.base.context.limiter.acquire().await;
                put_object(
                    &self.api.base.context.http,
                    &file_paths[i],
                    region,
                    &auth.secret_id,
                    &auth.secret_key,
                    &auth.token,
                    bucket_name,
                    object_key,
                )
                .await?;
            }

            finish_results.push(FinishUploadResultDict {
                storage: crate::models::helper::FinishUploadStorageDict {
                    bucket: crate::models::helper::FinishUploadBucketDict {
                        name: bucket_name.clone(),
                        region: region.clone(),
                    },
                    object_key: object_key.clone(),
                },
                upload_result: 0,
            });
        }

        let finish = self
            .api
            .finish_upload(&self.bus_id, &finish_results, self.credential.as_ref())
            .await?;
        finish
            .objects
            .ok_or_else(|| QmError::ApiData("FinishUpload 未返回上传成功的文件对象".into()))
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// 使用 COS 临时密钥执行 PUT Object 上传.
///
/// 签名采用 QCloud 旧版 `q-sign-algorithm=sha1` 方案 (与官方桌面客户端一致).
/// `http` 复用客户端的统一 HTTP 客户端 (共享代理/限流配置).
#[allow(clippy::too_many_arguments)]
async fn put_object(
    http: &reqwest::Client,
    file_path: &Path,
    region: &str,
    secret_id: &str,
    secret_key: &str,
    token: &str,
    bucket_name: &str,
    object_key: &str,
) -> Result<()> {
    let host = format!("{bucket_name}.cos.{region}.myqcloud.com");
    let url = format!("https://{host}/{object_key}");
    let path_uri = format!("/{object_key}");

    let key_time_start = now_secs() - 60;
    let key_time_end = now_secs() + 3600;
    let key_time = format!("{key_time_start};{key_time_end}");
    let sign_key = hmac_sha1_hex(secret_key.as_bytes(), key_time.as_bytes());

    let http_headers = format!("host={host}&x-cos-security-token={token}");
    let http_string = format!("put\n{path_uri}\n\n{http_headers}\n");
    let http_string_sha1 = sha1_hex(http_string.as_bytes());
    let string_to_sign = format!("sha1\n{key_time}\n{http_string_sha1}\n");
    let signature = hmac_sha1_hex(sign_key.as_bytes(), string_to_sign.as_bytes());

    let authorization = format!(
        "q-sign-algorithm=sha1&q-ak={secret_id}&q-sign-time={key_time}&q-key-time={key_time}&\
         q-header-list=host;x-cos-security-token&q-url-param-list=&q-signature={signature}"
    );

    let bytes = std::fs::read(file_path)
        .map_err(|e| QmError::Io(format!("读取文件失败 {file_path:?}: {e}")))?;
    let resp = http
        .put(&url)
        .header("Host", &host)
        .header("x-cos-security-token", token)
        .header("Authorization", authorization)
        .header("Content-Type", "application/octet-stream")
        .body(bytes)
        .send()
        .await
        .map_err(QmError::from)?;
    let status = resp.status().as_u16();
    if status != 200 {
        let body = resp.text().await.unwrap_or_default();
        return Err(QmError::http(status, body));
    }
    Ok(())
}

fn hmac_sha1_hex(key: &[u8], data: &[u8]) -> String {
    use hmac::{Mac, SimpleHmac};
    type HmacSha1 = SimpleHmac<sha1::Sha1>;
    let mut mac = HmacSha1::new_from_slice(key).expect("hmac key");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

fn sha1_hex(data: &[u8]) -> String {
    let mut hasher = sha1::Sha1::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}
