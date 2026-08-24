//! 多账号凭证管理: 持久化 (可插拔后端) + 过期自动刷新.
//!
//! `CredentialStore` 本身不决定凭证如何落盘: 持久化委托给
//! [`CredentialPersist`] 后端, 由宿主实现. 仓库内置
//! [`FileCredentialPersist`] 仅为开发便利 (明文 JSON), **不适合生产**;
//! 生产环境请实现安全存储后端 (系统 Keychain / 加密文件 / TPM 等).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
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
/// 凭证以明文 JSON 落盘, **不应**用于生产环境的账号存储. 在 Unix 上本实现会
/// 强制文件权限为 `0600`, 并使用同目录临时文件 + rename 原子替换；这只能降低
/// 同机其他用户误读及崩溃中断写入风险, **不等于加密存储**.
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
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            use std::time::{SystemTime, UNIX_EPOCH};

            let name = self
                .path
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("credentials.json");
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let temp_path = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), nonce));

            let result = (|| -> Result<()> {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&temp_path)
                    .map_err(QmError::from)?;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))
                    .map_err(QmError::from)?;
                file.write_all(data.as_bytes()).map_err(QmError::from)?;
                file.sync_all().map_err(QmError::from)?;
                std::fs::rename(&temp_path, &self.path).map_err(QmError::from)?;

                // 持久化目录项，降低掉电后 rename 丢失的窗口。部分特殊文件系统可能
                // 不允许打开目录，此时数据文件本身已安全写入，不把目录 fsync 作为硬失败。
                if let Ok(dir) = std::fs::File::open(parent) {
                    let _ = dir.sync_all();
                }
                Ok(())
            })();

            if result.is_err() {
                let _ = std::fs::remove_file(&temp_path);
            }
            return result;
        }

        #[cfg(not(unix))]
        {
            // Windows 等平台保留兼容写法；宿主生产环境仍应使用安全 CredentialPersist
            // 后端。跨平台“原子覆盖已有文件”语义并不由 std::fs::rename 一致保证。
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&self.path)
                .map_err(QmError::from)?;
            file.write_all(data.as_bytes()).map_err(QmError::from)?;
            file.sync_all().map_err(QmError::from)
        }
    }
}

/// 多账号凭证管理器.
///
/// Store 变更采用 copy-on-write：在持有 store 锁时创建候选快照，先成功持久化候选
/// 状态，再提交到内存。因此持久化失败不会留下“内存已改、磁盘未改”的半提交状态，
/// 且同一 Store 的持久化顺序与内存提交顺序一致。
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
    ///
    /// 此接口使用明文 JSON 文件后端, 仅适合开发/受控环境. 生产环境应使用
    /// [`CredentialStore::from_backend`] + 系统安全存储实现.
    pub fn load(path: &Path) -> Result<Self> {
        Self::from_backend(FileCredentialPersist::new(path))
    }

    /// 从自定义持久化后端加载凭证库.
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
    /// 注意: 明文 JSON 仅适合开发环境; 生产环境请使用 [`Self::with_backend`].
    pub fn with_path(mut self, path: &Path) -> Self {
        self.backend = Some(Box::new(FileCredentialPersist::new(path)));
        self
    }

    fn persist_store(&self, store: &Store) -> Result<()> {
        let data = serde_json::to_string_pretty(store).map_err(QmError::from)?;
        if let Some(backend) = &self.backend {
            backend.save(&data)?;
        }
        Ok(())
    }

    /// 主动持久化当前内存快照.
    pub fn save(&self) -> Result<()> {
        let store = self.store.lock().unwrap();
        self.persist_store(&store)
    }

    /// 添加或更新账号; 若库为空则自动设为当前账号.
    pub fn add(&self, credential: Credential) -> Result<()> {
        let mut store = self.store.lock().unwrap();
        let mut next = store.clone();
        if next.accounts.is_empty() {
            next.current = Some(credential.musicid);
        }
        next.accounts.insert(credential.musicid, credential);
        self.persist_store(&next)?;
        *store = next;
        Ok(())
    }

    /// 移除账号.
    pub fn remove(&self, musicid: i64) -> Result<bool> {
        let mut store = self.store.lock().unwrap();
        let mut next = store.clone();
        let removed = next.accounts.remove(&musicid).is_some();
        if next.current == Some(musicid) {
            next.current = next.accounts.keys().next().copied();
        }
        if removed {
            self.persist_store(&next)?;
            *store = next;
        }
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
        let mut store = self.store.lock().unwrap();
        if !store.accounts.contains_key(&musicid) {
            return Err(QmError::ValueError(format!("账号 {musicid} 不存在")));
        }
        if store.current == Some(musicid) {
            return Ok(());
        }
        let mut next = store.clone();
        next.current = Some(musicid);
        self.persist_store(&next)?;
        *store = next;
        Ok(())
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
    /// 网络刷新结束后会重新确认账号仍存在且没有被其他任务替换；否则不会把旧刷新
    /// 结果重新插入 Store，从而避免“刷新期间删除账号后又被复活”的竞态。
    pub async fn refresh(&self, client: &Client, musicid: i64) -> Result<Credential> {
        let lock = {
            let mut locks = self.refresh_locks.lock().unwrap();
            locks
                .entry(musicid)
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let guard = lock.lock().await;

        let result = self.refresh_locked(client, musicid).await;

        drop(guard);
        // 无等待者时清理 per-account lock，避免长期多账号使用造成 map 无界增长。
        let mut locks = self.refresh_locks.lock().unwrap();
        if std::sync::Arc::strong_count(&lock) == 2 {
            locks.remove(&musicid);
        }
        drop(locks);

        result
    }

    async fn refresh_locked(&self, client: &Client, musicid: i64) -> Result<Credential> {
        let credential = {
            let store = self.store.lock().unwrap();
            let credential = store
                .accounts
                .get(&musicid)
                .cloned()
                .ok_or_else(|| QmError::CredentialInvalid(format!("账号 {musicid} 不存在")))?;
            if !credential.is_expired() {
                return Ok(credential);
            }
            credential
        };

        let refreshed = client.login.refresh_credential(Some(&credential)).await?;

        {
            let mut store = self.store.lock().unwrap();
            let current_value = store.accounts.get(&musicid).cloned().ok_or_else(|| {
                QmError::CredentialInvalid(format!("账号 {musicid} 已在刷新期间被移除"))
            })?;

            // 若账号在网络请求期间被另一个操作更新，旧 refresh 结果不能覆盖新状态。
            if current_value.musickey != credential.musickey
                || current_value.refresh_token != credential.refresh_token
            {
                return Ok(current_value);
            }

            let mut next = store.clone();
            next.accounts.insert(musicid, refreshed.clone());
            self.persist_store(&next)?;
            *store = next;
        }

        client.context().invalidate_session(musicid).await;
        Ok(refreshed)
    }

    /// 确保当前凭证有效: 过期时自动刷新, 并将生效凭证同步回 `client`.
    ///
    /// 若并发切换当前账号，本方法会重新读取新的 current，而不会在旧账号刷新完成后
    /// 把 Client 切回旧账号。
    pub async fn ensure_current(&self, client: &Client) -> Result<Credential> {
        const MAX_CURRENT_RETRIES: usize = 4;

        for _ in 0..MAX_CURRENT_RETRIES {
            let musicid =
                self.store.lock().unwrap().current.ok_or_else(|| {
                    QmError::CredentialInvalid("凭证库为空, 请先 add 账号".into())
                })?;

            let effective = if self.is_expired(musicid) {
                self.refresh(client, musicid).await?
            } else {
                self.get(musicid)
                    .ok_or_else(|| QmError::CredentialInvalid(format!("账号 {musicid} 不存在")))?
            };

            let still_current = self.store.lock().unwrap().current == Some(musicid);
            if still_current {
                client.set_credential(effective.clone());
                return Ok(effective);
            }
        }

        Err(QmError::Protocol {
            stage: "credential-store",
            message: "current account changed repeatedly while ensuring credentials".into(),
        })
    }

    /// 将当前账号应用到客户端.
    pub fn apply_current(&self, client: &Client) -> Result<()> {
        let credential = self
            .current()
            .ok_or_else(|| QmError::CredentialInvalid("凭证库为空".into()))?;
        client.set_credential(credential);
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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        let loaded = CredentialStore::load(&dir).unwrap();
        assert_eq!(loaded.get(10001).map(|c| c.musickey), Some("key-1".into()));
        assert_eq!(loaded.current().map(|c| c.musicid), Some(10001));

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
    fn persistence_failure_does_not_commit_memory() {
        #[derive(Debug)]
        struct FailingBackend;
        impl CredentialPersist for FailingBackend {
            fn load(&self) -> Result<Option<String>> {
                Ok(None)
            }
            fn save(&self, _data: &str) -> Result<()> {
                Err(QmError::Io("forced failure".into()))
            }
        }

        let store = CredentialStore::new().with_backend(FailingBackend);
        let result = store.add(Credential {
            musicid: 42,
            str_musicid: "42".into(),
            musickey: "secret".into(),
            ..Default::default()
        });
        assert!(result.is_err());
        assert!(store.get(42).is_none());
        assert!(store.current().is_none());
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
        assert!(store.is_expired(1));
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
        let data = backend.inner.lock().unwrap().clone().unwrap();
        assert!(data.contains("secret"));
        let loaded = CredentialStore::from_backend(backend).unwrap();
        assert_eq!(loaded.get(7).map(|c| c.musickey), Some("secret".into()));
    }

    #[test]
    fn account_ids_are_stable_sorted() {
        let store = CredentialStore::new();
        for id in [300, 1, 200] {
            store
                .add(Credential {
                    musicid: id,
                    str_musicid: id.to_string(),
                    ..Default::default()
                })
                .unwrap();
        }
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
        let effective = store.ensure_current(&client).await.unwrap();
        assert_eq!(effective.musicid, 9);
        assert_eq!(client.credential().musicid, 9);
        assert_eq!(client.credential().musickey, "k9");
    }

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
