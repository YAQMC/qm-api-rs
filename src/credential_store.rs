//! 多账号凭证管理: 持久化 + 过期自动刷新.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use crate::error::{QmError, Result};
use crate::models::Credential;
use crate::Client;

/// 持久化的账号表.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    accounts: BTreeMap<i64, Credential>,
    #[serde(default)]
    current: Option<i64>,
}

/// 多账号凭证管理器.
///
/// - 按 `musicid` 区分账号.
/// - `save` / `load` 持久化到磁盘 (JSON).
/// - `refresh_current` 在凭证过期时通过 `login.refresh_credential` 自动刷新.
#[derive(Debug)]
pub struct CredentialStore {
    store: Mutex<Store>,
    path: Option<std::path::PathBuf>,
}

impl Default for CredentialStore {
    fn default() -> Self {
        CredentialStore {
            store: Mutex::new(Store::default()),
            path: None,
        }
    }
}

impl CredentialStore {
    /// 创建空凭证库.
    pub fn new() -> Self {
        CredentialStore::default()
    }

    /// 从文件加载凭证库.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| QmError::Io(e.to_string()))?;
        let store: Store = serde_json::from_str(&text).map_err(QmError::from)?;
        Ok(CredentialStore {
            store: Mutex::new(store),
            path: Some(path.to_path_buf()),
        })
    }

    /// 设置持久化路径 (后续 `save` 自动写入).
    pub fn with_path(mut self, path: &Path) -> Self {
        self.path = Some(path.to_path_buf());
        self
    }

    /// 持久化到磁盘 (需先设置 path).
    pub fn save(&self) -> Result<()> {
        let path = self.path.as_ref().ok_or_else(|| QmError::ValueError("未设置持久化路径".into()))?;
        let store = self.store.lock().unwrap();
        let bytes = serde_json::to_vec_pretty(&*store).map_err(QmError::from)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, bytes).map_err(QmError::from)
    }

    /// 添加或更新账号; 若库为空则自动设为当前账号.
    pub fn add(&self, credential: Credential) -> Result<()> {
        let mut store = self.store.lock().unwrap();
        if store.accounts.is_empty() {
            store.current = Some(credential.musicid);
        }
        store.accounts.insert(credential.musicid, credential);
        self.persist(&store)?;
        Ok(())
    }

    /// 移除账号.
    pub fn remove(&self, musicid: i64) -> Result<bool> {
        let mut store = self.store.lock().unwrap();
        let removed = store.accounts.remove(&musicid).is_some();
        if store.current == Some(musicid) {
            store.current = store.accounts.keys().next().copied();
        }
        self.persist(&store)?;
        Ok(removed)
    }

    /// 获取指定账号凭证.
    pub fn get(&self, musicid: i64) -> Option<Credential> {
        self.store.lock().unwrap().accounts.get(&musicid).cloned()
    }

    /// 获取当前账号凭证.
    pub fn current(&self) -> Option<Credential> {
        let store = self.store.lock().unwrap();
        store.current.and_then(|id| store.accounts.get(&id).cloned())
    }

    /// 设置当前账号.
    pub fn set_current(&self, musicid: i64) -> Result<()> {
        let mut store = self.store.lock().unwrap();
        if !store.accounts.contains_key(&musicid) {
            return Err(QmError::ValueError(format!("账号 {musicid} 不存在")));
        }
        store.current = Some(musicid);
        self.persist(&store)?;
        Ok(())
    }

    /// 所有账号的 musicid 列表.
    pub fn account_ids(&self) -> Vec<i64> {
        self.store.lock().unwrap().accounts.keys().copied().collect()
    }

    /// 检查凭证是否过期.
    pub fn is_expired(&self, musicid: i64) -> bool {
        self.get(musicid).map(|c| c.is_expired()).unwrap_or(false)
    }

    /// 刷新指定账号凭证 (自动持久化).
    pub async fn refresh(&self, client: &Client, musicid: i64) -> Result<Credential> {
        let credential = self
            .get(musicid)
            .ok_or_else(|| QmError::CredentialInvalid(format!("账号 {musicid} 不存在")))?;
        let refreshed = client.login.refresh_credential(Some(&credential)).await?;
        let mut store = self.store.lock().unwrap();
        store.accounts.insert(musicid, refreshed.clone());
        self.persist(&store)?;
        Ok(refreshed)
    }

    /// 确保当前凭证有效: 过期时自动刷新.
    ///
    /// 返回当前有效凭证; 无账号或刷新失败时返回错误.
    pub async fn ensure_current(&self, client: &Client) -> Result<Credential> {
        let musicid = self
            .store
            .lock()
            .unwrap()
            .current
            .ok_or_else(|| QmError::CredentialInvalid("凭证库为空, 请先 add 账号".into()))?;
        if self.is_expired(musicid) {
            self.refresh(client, musicid).await
        } else {
            Ok(self.get(musicid).expect("account exists"))
        }
    }

    /// 将当前账号应用到客户端.
    pub fn apply_current(&self, client: &Client) -> Result<()> {
        let credential = self
            .current()
            .ok_or_else(|| QmError::CredentialInvalid("凭证库为空".into()))?;
        client.set_credential(credential);
        Ok(())
    }

    fn persist(&self, store: &Store) -> Result<()> {
        if let Some(path) = &self.path {
            let bytes = serde_json::to_vec_pretty(store).map_err(QmError::from)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, bytes).map_err(QmError::from)?;
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
}
