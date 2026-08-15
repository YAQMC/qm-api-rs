//! 辅助功能 API (对应 Python 端 `modules/helper.py`).
//!
//! 提供 COS 上传初始化与完成的辅助接口, 以及客户端更新检查等杂项接口.

use serde_json::{json, Value};

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::helper::*;
use crate::models::Credential;

/// 辅助功能 API.
#[derive(Clone, Debug)]
pub struct HelperApi {
    pub(crate) base: ApiModule,
}

impl HelperApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        HelperApi {
            base: ApiModule::new(context),
        }
    }

    /// 初始化 COS 上传以获取临时凭证.
    ///
    /// `files` 每项需包含 `FileSha1` / `FileName` / `FileSize`.
    pub async fn init_upload(
        &self,
        bus_id: &str,
        files: &[InitUploadFileDict],
        credential: Option<&Credential>,
    ) -> Result<InitUploadResponse> {
        let files: Vec<Value> = files
            .iter()
            .map(|f| {
                json!({
                    "FileSha1": f.file_sha1,
                    "FileName": f.file_name,
                    "FileSize": f.file_size,
                })
            })
            .collect();
        let mut opts = RequestOptions::default();
        opts.sign = true;
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.filesys.FileSystem",
                "InitUpload",
                json!({ "BusID": bus_id, "Files": files }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 完成 COS 上传并通知服务器验证.
    pub async fn finish_upload(
        &self,
        bus_id: &str,
        results: &[FinishUploadResultDict],
        credential: Option<&Credential>,
    ) -> Result<FinishUploadResponse> {
        let results: Vec<Value> = serde_json::to_value(results)
            .map_err(|e| crate::error::QmError::Deserialize(e.to_string()))?
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut opts = RequestOptions::default();
        opts.sign = true;
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.filesys.FileSystem",
                "FinishUpload",
                json!({ "BusID": bus_id, "Results": results }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 检查客户端更新 (官方桌面客户端 `platform.uniteUpdate.UniteUpdateSvr`).
    pub async fn query_update(&self, cv: i64) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.comm = Some(json!({ "ct": 31, "cv": cv }));
        self.base
            .cgi("platform.uniteUpdate.UniteUpdateSvr", "QueryUpdate", json!({}), opts)
            .await
    }
}
