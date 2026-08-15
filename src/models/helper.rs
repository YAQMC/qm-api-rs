//! Helper API 返回模型定义 (对应 Python 端 `models/helper.py`).

use serde::{Deserialize, Serialize};

/// InitUpload 的单文件参数字典.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InitUploadFileDict {
    #[serde(rename = "FileSha1")]
    pub file_sha1: String,
    #[serde(rename = "FileName")]
    pub file_name: String,
    #[serde(rename = "FileSize")]
    pub file_size: i64,
}

/// FinishUpload 的 Bucket 字典.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FinishUploadBucketDict {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Region")]
    pub region: String,
}

/// FinishUpload 的 Storage 字典.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FinishUploadStorageDict {
    #[serde(rename = "Bucket")]
    pub bucket: FinishUploadBucketDict,
    #[serde(rename = "ObjectKey")]
    pub object_key: String,
}

/// FinishUpload 的单文件结果字典.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FinishUploadResultDict {
    #[serde(rename = "Storage")]
    pub storage: FinishUploadStorageDict,
    #[serde(rename = "UploadResult")]
    pub upload_result: i64,
}

/// COS 存储桶基本信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UploadBucketInfo {
    #[serde(alias = "Name")]
    pub name: String,
    #[serde(alias = "Region")]
    pub region: String,
}

/// COS 存储桶上传状态.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UploadBucketStatus {
    #[serde(alias = "Bucket")]
    pub bucket: UploadBucketInfo,
    #[serde(alias = "UploadStatus")]
    pub upload_status: i64,
}

/// COS 上传文件元信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UploadFileInfo {
    #[serde(alias = "FileSha1")]
    pub file_sha1: String,
    #[serde(alias = "ObjectKey")]
    pub object_key: String,
    #[serde(alias = "Buckets")]
    pub buckets: Vec<UploadBucketStatus>,
}

/// COS 上传鉴权信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UploadAuthInfo {
    #[serde(alias = "SecretID")]
    pub secret_id: String,
    #[serde(alias = "SecretKey")]
    pub secret_key: String,
    #[serde(alias = "Token")]
    pub token: String,
    #[serde(alias = "StartTime")]
    pub start_time: i64,
    #[serde(alias = "ExpiredTime")]
    pub expired_time: i64,
}

/// InitUpload 接口返回数据.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct InitUploadResponse {
    #[serde(alias = "AuthInfo")]
    pub auth_info: UploadAuthInfo,
    #[serde(alias = "Files")]
    pub files: Vec<UploadFileInfo>,
}

/// 存储信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UploadStorage {
    #[serde(alias = "Bucket")]
    pub bucket: UploadBucketInfo,
    #[serde(alias = "ObjectKey")]
    pub object_key: String,
}

/// COS 上传完成后的 URL 详情.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UploadUrlInfo {
    #[serde(alias = "FileId")]
    pub file_id: String,
    #[serde(alias = "URL")]
    pub url: String,
    #[serde(alias = "CDNURL")]
    pub cdn_url: String,
    #[serde(alias = "PresignedURL")]
    pub presigned_url: String,
    #[serde(alias = "InternalURL")]
    pub internal_url: String,
}

/// COS 上传完成后的文件对象.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UploadObjectInfo {
    #[serde(alias = "Storage")]
    pub storage: UploadStorage,
    #[serde(alias = "Url")]
    pub url: UploadUrlInfo,
}

/// FinishUpload 接口返回数据.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FinishUploadResponse {
    #[serde(alias = "Objects")]
    pub objects: Option<Vec<UploadObjectInfo>>,
}
