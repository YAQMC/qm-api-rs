//! 多账号凭证管理: 持久化 (可插拔后端) + 过期自动刷新.
//!
//! `CredentialStore` 本身不决定凭证如何落盘: 持久化委托给
//! [`CredentialPersist`] 后端, 由宿主实现. 仓库内置
//! [`FileCredentialPersist`] 仅为开发便利 (明文 JSON), **不适合生产**;
//! 生产环境请实现安全存储后端 (系统 Keychain / 加密文件 / TPM 等).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use crate::error::{QmError, Result};
use crate::models::Credential;
use crate::Client;

/// 持久化的账号表.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    accounts: BTreeMap<i64, Credential>,
    #[serde(default)]
    current: Option<i64>,
}

/// 凭证持久化后端.
///
/// 由宿主实现, 例如使用系统安全存储 / Keychain / 加密文件. 默认的
/// [`FileCredentialPersist`] 为明文 JSON, 仅适合开发环境.
pub trait CredentialPersist: Send + Sync + std::fmt::Debug {
    /// 读取序列化数据 (文件不存在时返回 `Ok(None)`).
    fn load(&self) -> Result<Option<String>>;
    /// 写入序列化数据.
    fn save(&self, data: &str) -> Result<()>;
}

/// 明文 JSON 文件后端 (仅开发环境).
///
/// 凭证以明文 JSON 落盘, **不应**用于生产环境的账号存储.
#[derive(Debug, Clone)]
pub struct FileCredentialPersist {
    path: std::path::PathBuf,
}

impl FileCredentialPersist {
    /// 指定持久化文件路径.
    pub fn new(path: &Path) -> Self {
        FileCredentialPersist {
            path: path.to_path_buf(),
        }
    }

    /// 当前持久化路径.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl CredentialPersist for FileCredentialPersist {
    fn load(&self) -> Result<Option<String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(QmError::Io(e.to_string())),
        }
    }

    fn save(&self, data: &str) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, data).map_err(QmError::from)
    }
}

/// 多账号凭证管理器.
///
/// - 按 `musicid` 区分账号.
/// - 持久化通过可插拔的 [`CredentialPersist`] 后端 (默认文件明文后端仅开发用).
/// - `refresh_current` 在凭证过期时通过 `login.refresh_credential` 自动刷新.
///
/// 出于安全考虑, `Credential` 的 `Debug` 已对令牌字段做 redaction;
/// 生产环境的账号存储应由宿主自行实现安全后端.
#[derive(Debug)]
pub struct CredentialStore {
    store: Mutex<Store>,
    backend: Option<Box<dyn CredentialPersist>>,
    /// 按账号的刷新锁 (避免并发 refresh_token 请求).
    refresh_locks:
        std::sync::Mutex<std::collections::HashMap<i64, std::sync::Arc<tokio::sync::Mutex<()>>>>,
}

impl Default for CredentialStore {
    fn default() -> Self {
        CredentialStore {
            store: Mutex::new(Store::default()),
            backend: None,
            refresh_locks: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl CredentialStore {
    /// 创建空凭证库 (仅内存, 不持久化).
    pub fn new() -> Self {
        CredentialStore::default()
    }

    /// 从文件加载凭证库 (同时使用该文件作为持久化后端).
    pub fn load(path: &Path) -> Result<Self> {
        Self::from_backend(FileCredentialPersist::new(path))
    }

    /// 从自定义持久化后端加载凭证库.
    ///
    /// 后端已有数据时恢复; 无数据时得到空库. 后续变更自动写入该后端.
    pub fn from_backend(backend: impl CredentialPersist + 'static) -> Result<Self> {
        let backend = Box::new(backend);
        let store = match backend.load()? {
            Some(text) => serde_json::from_str(&text).map_err(QmError::from)?,
            None => Store::default(),
        };
        Ok(CredentialStore {
            store: Mutex::new(store),
            backend: Some(backend),
            refresh_locks: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// 使用自定义持久化后端 (空库开始, 后续变更自动写入该后端).
    pub fn with_backend(mut self, backend: impl CredentialPersist + 'static) -> Self {
        self.backend = Some(Box::new(backend));
        self
    }

    /// 设置明文文件持久化后端 (后续 `save` 自动写入).
    ///
    /// 注意: 明文 JSON 仅适合开发环境.
    pub fn with_path(mut self, path: &Path) -> Self {
        self.backend = Some(Box::new(FileCredentialPersist::new(path)));
        self
    }

    /// 主动持久化到后端 (需先设置 `with_path` / `with_backend`).
    pub fn save(&self) -> Result<()> {
        let store = self.store.lock().unwrap().clone();
        let data = serde_json::to_string_pretty(&store).map_err(QmError::from)?;
        if let Some(backend) = &self.backend {
            backend.save(&data)?;
        }
        Ok(())
    }

    /// 添加或更新账号; 若库为空则自动设为当前账号.
    pub fn add(&self, credential: Credential) -> Result<()> {
        {
            let mut store = self.store.lock().unwrap();
            if store.accounts.is_empty() {
                store.current = Some(credential.musicid);
            }
            store.accounts.insert(credential.musicid, credential);
        }
        self.persist()
    }

    /// 移除账号.
    pub fn remove(&self, musicid: i64) -> Result<bool> {
        let removed = {
            let mut store = self.store.lock().unwrap();
            let removed = store.accounts.remove(&musicid).is_some();
            if store.current == Some(musicid) {
                store.current = store.accounts.keys().next().copied();
            }
            removed
        };
        self.persist()?;
        Ok(removed)
    }

    /// 获取指定账号凭证.
    pub fn get(&self, musicid: i64) -> Option<Credential> {
        self.store.lock().unwrap().accounts.get(&musicid).cloned()
    }

    /// 获取当前账号凭证.
    pub fn current(&self) -> Option<Credential> {
        let store = self.store.lock().unwrap();
        store
            .current
            .and_then(|id| store.accounts.get(&id).cloned())
    }

    /// 设置当前账号.
    pub fn set_current(&self, musicid: i64) -> Result<()> {
        {
            let mut store = self.store.lock().unwrap();
            if !store.accounts.contains_key(&musicid) {
                return Err(QmError::ValueError(format!("账号 {musicid} 不存在")));
            }
            store.current = Some(musicid);
        }
        self.persist()
    }

    /// 所有账号的 musicid 列表.
    pub fn account_ids(&self) -> Vec<i64> {
        self.store
            .lock()
            .unwrap()
            .accounts
            .keys()
            .copied()
            .collect()
    }

    /// 检查凭证是否过期.
    pub fn is_expired(&self, musicid: i64) -> bool {
        self.get(musicid).map(|c| c.is_expired()).unwrap_or(false)
    }

    /// 刷新指定账号凭证 (自动持久化, per-account singleflight).
    ///
    /// 锁内会重新检查凭证是否仍过期: 若已被其他并发任务刷新则直接返回,
    /// 避免对可能 one-time 的 refresh_token 发出重复请求.
    pub async fn refresh(&self, client: &Client, musicid: i64) -> Result<Credential> {
        let lock = {
            let mut locks = self.refresh_locks.lock().unwrap();
            locks
                .entry(musicid)
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;
        // 锁内复查: 已被其他任务刷新则直接返回.
        if !self.is_expired(musicid) {
            return self
                .get(musicid)
                .ok_or_else(|| QmError::CredentialInvalid(format!("账号 {musicid} 不存在")));
        }
        let credential = self
            .get(musicid)
            .ok_or_else(|| QmError::CredentialInvalid(format!("账号 {musicid} 不存在")))?;
        let refreshed = client.login.refresh_credential(Some(&credential)).await?;
        {
            let mut store = self.store.lock().unwrap();
            store.accounts.insert(musicid, refreshed.clone());
        }
        self.persist()?;
        // 凭证已刷新: 使该账号的 Android session 失效 (旧鉴权下申请的 session 保守作废).
        client.context().invalidate_session(musicid).await;
        Ok(refreshed)
    }

    /// 确保当前凭证有效: 过期时自动刷新, 并将生效凭证同步回 `client`.
    ///
    /// 返回当前有效凭证; 无账号或刷新失败时返回错误.
    pub async fn ensure_current(&self, client: &Client) -> Result<Credential> {
        let musicid = self
            .store
            .lock()
            .unwrap()
            .current
            .ok_or_else(|| QmError::CredentialInvalid("凭证库为空, 请先 add 账号".into()))?;
        let effective = if self.is_expired(musicid) {
            self.refresh(client, musicid).await?
        } else {
            self.get(musicid)
                .ok_or_else(|| QmError::CredentialInvalid(format!("账号 {musicid} 不存在")))?
        };
        // 保证 Client 与 Store 同步: 刷新后立即把新凭证写回 Client,
        // 避免后续未显式传 credential 的 API 继续使用旧 token.
        client.set_credential(effective.clone());
        Ok(effective)
    }

    /// 将当前账号应用到客户端.
    pub fn apply_current(&self, client: &Client) -> Result<()> {
        let credential = self
            .current()
            .ok_or_else(|| QmError::CredentialInvalid("凭证库为空".into()))?;
        client.set_credential(credential);
        Ok(())
    }

    fn persist(&self) -> Result<()> {
        let store = self.store.lock().unwrap().clone();
        let data = serde_json::to_string_pretty(&store).map_err(QmError::from)?;
        if let Some(backend) = &self.backend {
            backend.save(&data)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_roundtrip() {
        let dir = std::env::temp_dir().join("qm_store_test.json");
        let _ = std::fs::remove_file(&dir);

        let store = CredentialStore::new().with_path(&dir);
        let cred = Credential {
            musicid: 10001,
            str_musicid: "10001".into(),
            musickey: "key-1".into(),
            login_type: 2,
            ..Default::default()
        };
        store.add(cred.clone()).unwrap();
        assert_eq!(store.current().map(|c| c.musicid), Some(10001));

        // 重新加载.
        let loaded = CredentialStore::load(&dir).unwrap();
        assert_eq!(loaded.get(10001).map(|c| c.musickey), Some("key-1".into()));
        assert_eq!(loaded.current().map(|c| c.musicid), Some(10001));

        // 多账号.
        let cred2 = Credential {
            musicid: 20002,
            str_musicid: "20002".into(),
            musickey: "key-2".into(),
            login_type: 1,
            ..Default::default()
        };
        loaded.add(cred2).unwrap();
        loaded.set_current(20002).unwrap();
        assert_eq!(loaded.account_ids(), vec![10001, 20002]);
        assert_eq!(loaded.current().map(|c| c.musicid), Some(20002));

        loaded.remove(10001).unwrap();
        assert_eq!(loaded.account_ids(), vec![20002]);
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn expired_check() {
        let store = CredentialStore::new();
        let cred = Credential {
            musicid: 1,
            str_musicid: "1".into(),
            musickey: "k".into(),
            musickey_create_time: 1,
            key_expires_in: 5,
            ..Default::default()
        };
        store.add(cred).unwrap();
        // create=1, expires=5 -> 早已过期.
        assert!(store.is_expired(1));
        // key_expires_in=0 视为无过期信息, 不算过期.
        let cred2 = Credential {
            musicid: 2,
            str_musicid: "2".into(),
            musickey: "k2".into(),
            musickey_create_time: 1,
            key_expires_in: 0,
            ..Default::default()
        };
        store.add(cred2).unwrap();
        assert!(!store.is_expired(2));
    }

    #[test]
    fn custom_backend_is_used() {
        let backend = InMemoryBackend::default();
        let store = CredentialStore::new().with_backend(backend.clone());
        store
            .add(Credential {
                musicid: 7,
                str_musicid: "7".into(),
                musickey: "secret".into(),
                ..Default::default()
            })
            .unwrap();
        // 数据写入后端 (而非文件).
        let data = backend.inner.lock().unwrap().clone().unwrap();
        assert!(data.contains("secret"));

        // 从后端数据恢复.
        let loaded = CredentialStore::from_backend(backend).unwrap();
        assert_eq!(loaded.get(7).map(|c| c.musickey), Some("secret".into()));
    }

    #[test]
    fn account_ids_are_stable_sorted() {
        let store = CredentialStore::new();
        store
            .add(Credential {
                musicid: 300,
                str_musicid: "300".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .add(Credential {
                musicid: 1,
                str_musicid: "1".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .add(Credential {
                musicid: 200,
                str_musicid: "200".into(),
                ..Default::default()
            })
            .unwrap();
        // 底层为 BTreeMap, 迭代顺序确定 (升序), 不依赖插入顺序.
        assert_eq!(store.account_ids(), vec![1, 200, 300]);
    }

    #[tokio::test]
    async fn ensure_current_syncs_client() {
        let store = CredentialStore::new();
        let cred = Credential {
            musicid: 9,
            str_musicid: "9".into(),
            musickey: "k9".into(),
            login_type: 2,
            ..Default::default()
        };
        store.add(cred).unwrap();

        let client = crate::Client::new(None, None).unwrap();
        // 未过期路径: ensure_current 也把生效凭证写回 Client.
        let effective = store.ensure_current(&client).await.unwrap();
        assert_eq!(effective.musicid, 9);
        assert_eq!(client.credential().musicid, 9);
        assert_eq!(client.credential().musickey, "k9");
    }

    /// 内存后端 (测试用).
    #[derive(Debug, Clone, Default)]
    struct InMemoryBackend {
        inner: std::sync::Arc<Mutex<Option<String>>>,
    }

    impl CredentialPersist for InMemoryBackend {
        fn load(&self) -> Result<Option<String>> {
            Ok(self.inner.lock().unwrap().clone())
        }

        fn save(&self, data: &str) -> Result<()> {
            *self.inner.lock().unwrap() = Some(data.to_string());
            Ok(())
        }
    }
}
