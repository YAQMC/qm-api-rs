//! Account-scoped reads and typed writes used by desktop and mobile hosts.
//!
//! Endpoint and credential selection live here. `AccountPage` keeps a bounded
//! compatibility envelope while hosts migrate their legacy song/playlist DTOs.
//! Offsets count upstream rows, not successfully rendered songs.

use serde_json::{json, Value};

use crate::{
    CancellationToken, CgiOptions, Client, Credential, HttpMethod, HttpOptions, QmError, Result,
};

/// The only account read operations exposed by this compatibility boundary.
#[derive(Clone, Debug)]
pub enum AccountRead {
    FavoriteSongs,
    OwnedPlaylists,
    CollectedPlaylists,
    PlaylistTracks {
        tid: String,
    },
    /// Owned DissInfo can identify a directory instead of repeating its public
    /// playlist TID. The caller must obtain this binding from the same account's
    /// trusted playlist listing, not from the response being validated.
    OwnedPlaylistTracks {
        tid: String,
        dir_id: u64,
    },
    RecentlyPlayed,
}

/// Typed account mutations supported by the provider boundary.
#[derive(Clone, Debug)]
pub enum AccountWrite {
    FavoriteSong {
        add: bool,
        song_id: i64,
        song_type: i64,
    },
    PlaylistTracks {
        add: bool,
        dir_id: i64,
        tid: i64,
        songs: Vec<(i64, i64)>,
    },
    CreatePlaylist {
        name: String,
    },
    DeletePlaylist {
        dir_id: i64,
    },
    EditPlaylist {
        dir_id: i64,
        mask: i64,
        name: String,
        description: String,
        picture_url: String,
        tag_list: String,
    },
    CollectPlaylist {
        collect: bool,
        playlist_id: i64,
    },
}

/// Validated pagination information with lossless legacy metadata.
pub struct AccountPage {
    envelope: Value,
    pub next_offset: Option<u64>,
    pub total: Option<u64>,
    pub row_count: usize,
}

impl std::fmt::Debug for AccountPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountPage")
            .field("next_offset", &self.next_offset)
            .field("total", &self.total)
            .field("row_count", &self.row_count)
            .finish_non_exhaustive()
    }
}

impl AccountPage {
    /// Transitional DTO mapping only; callers must not use this to issue CGI.
    pub fn compatibility_json(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(&self.envelope)?)
    }
}

/// Read one bounded page for an explicit account. This never mutates/inherits
/// the Client's default identity and never converts an ordinary UIN to EUIN.
pub async fn read_page(
    client: &Client,
    credential: &Credential,
    operation: AccountRead,
    offset: u64,
    limit: u32,
    cancellation: CancellationToken,
) -> Result<AccountPage> {
    if cancellation.is_cancelled() {
        return Err(cancelled());
    }
    if !(1..=100).contains(&limit) || offset.checked_add(u64::from(limit)).is_none() {
        return Err(QmError::ValueError("invalid account page bounds".into()));
    }
    if credential.musicid <= 0 || credential.musickey.is_empty() {
        return Err(QmError::CredentialInvalid(
            "account read requires credentials".into(),
        ));
    }
    let uin = credential.str_musicid();
    let (module, method, param) = match &operation {
        AccountRead::OwnedPlaylists => (
            "music.musicasset.PlaylistBaseRead",
            "GetPlaylistByUin",
            json!({"uin":uin,"sin":offset,"ein":offset + u64::from(limit) - 1}),
        ),
        AccountRead::CollectedPlaylists => {
            if credential.encrypt_uin.trim().is_empty() {
                return Err(QmError::CredentialInvalid(
                    "collected playlists require encrypted account identity".into(),
                ));
            }
            (
                "music.musicasset.PlaylistFavRead",
                "CgiGetPlaylistFavInfo",
                json!({"uin":credential.encrypt_uin,"offset":offset,"size":limit}),
            )
        }
        AccountRead::FavoriteSongs
        | AccountRead::PlaylistTracks { .. }
        | AccountRead::OwnedPlaylistTracks { .. } => {
            let mut param = json!({
                "disstid": 0, "dirid": 201, "tag": true,
                "song_begin": offset, "song_num": limit,
                "userinfo": true, "orderlist": true, "onlysonglist": 1
            });
            match &operation {
                AccountRead::PlaylistTracks { tid }
                | AccountRead::OwnedPlaylistTracks { tid, .. } => {
                    if matches!(
                        &operation,
                        AccountRead::OwnedPlaylistTracks { dir_id: 0, .. }
                    ) {
                        return Err(QmError::ValueError(
                            "invalid owned playlist directory".into(),
                        ));
                    }
                    if tid.is_empty()
                        || tid.len() > 128
                        || !tid
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                    {
                        return Err(QmError::ValueError("invalid playlist identifier".into()));
                    }
                    param["disstid"] = tid
                        .parse::<u64>()
                        .map(Value::from)
                        .unwrap_or_else(|_| Value::String(tid.clone()));
                    param["dirid"] = json!(0);
                }
                AccountRead::FavoriteSongs if !credential.encrypt_uin.is_empty() => {
                    param["enc_host_uin"] = json!(credential.encrypt_uin);
                }
                _ => {}
            }
            ("music.srfDissInfo.DissInfo", "CgiGetDiss", param)
        }
        AccountRead::RecentlyPlayed => (
            "music.musichallSong.RecentPlayList",
            "GetRecentPlayList",
            json!({ "uin": uin, "begin": offset, "num": limit }),
        ),
    };
    let gtk = crate::hash33(&credential.musickey, 5381);
    // Preserve the host's validated account-read envelope (not Android session
    // negotiation and not the account-write identity envelope).
    let payload = json!({
        "comm": {
            "ct":24, "cv":4_747_474, "format":"json", "inCharset":"utf-8",
            "outCharset":"utf-8", "notice":0, "needNewCode":1,
            "platform":"yqq.json", "uin":uin,
            "g_tk":gtk, "g_tk_new_20200303":gtk
        },
        "req": { "module": module, "method": method, "param": param }
    });
    let opts = HttpOptions {
        credential: Some(credential.clone()),
        json: Some(payload),
        headers: vec![
            ("Origin".into(), "https://y.qq.com".into()),
            ("Referer".into(), "https://y.qq.com/".into()),
        ],
        cancellation: cancellation.clone(),
        ..HttpOptions::default()
    };
    let response = client
        .request_http(
            HttpMethod::Post,
            "https://u.y.qq.com/cgi-bin/musicu.fcg",
            &opts,
        )
        .await?;
    if cancellation.is_cancelled() {
        return Err(cancelled());
    }
    parse_page(serde_json::from_str(&response)?, &operation, offset, limit)
}

// Private wire executor: external callers select an AccountWrite, never a
// module/method pair or unchecked JSON parameters.
async fn execute_write(
    client: &Client,
    credential: &Credential,
    module: &str,
    method: &str,
    param: Value,
    cancellation: CancellationToken,
) -> Result<crate::CgiReply<Value>> {
    let allowed = matches!(
        (module, method),
        ("music.musicasset.PlaylistDetailWrite", "AddSonglist")
            | ("music.musicasset.PlaylistDetailWrite", "DelSonglist")
            | ("music.musicasset.PlaylistBaseWrite", "AddPlaylist")
            | ("music.musicasset.PlaylistBaseWrite", "DelPlaylist")
            | ("music.musicasset.PlaylistBaseWrite", "EditPlaylist")
            | ("music.musicasset.PlaylistFavWrite", "FavPlaylist")
            | ("music.musicasset.PlaylistFavWrite", "CancelFavPlaylist")
    );
    if !allowed {
        return Err(QmError::ValueError(
            "unsupported account write endpoint".into(),
        ));
    }
    if credential.musicid <= 0 || credential.musickey.is_empty() {
        return Err(QmError::CredentialInvalid(
            "account write requires credentials".into(),
        ));
    }
    let mut options = CgiOptions::default();
    options.comm = Some(account_write_comm(credential));
    options.override_comm = true;
    options.credential = Some(credential.clone());
    // This envelope is the Web account contract even on an Android Client.
    // Do not negotiate an unrelated Android session before a bounded write.
    options.platform = Some(crate::Platform::Web);
    options.require_login = true;
    options.retry = crate::RetryClass::Write;
    options.preserve_bool = true;
    options.cancellation = cancellation;
    client.request_cgi(module, method, param, &options).await
}

/// Execute a typed account mutation. Endpoint names and wire parameters are
/// selected exclusively inside qm-api-rs.
pub async fn write(
    client: &Client,
    credential: &Credential,
    operation: AccountWrite,
    cancellation: CancellationToken,
) -> Result<crate::CgiReply<Value>> {
    if cancellation.is_cancelled() {
        return Err(cancelled());
    }
    let (module, method, param) = operation.into_wire(credential)?;
    let reply = execute_write(
        client,
        credential,
        module,
        method,
        param,
        cancellation.clone(),
    )
    .await?;
    if cancellation.is_cancelled() {
        return Err(cancelled());
    }
    Ok(reply)
}

impl AccountWrite {
    fn into_wire(self, credential: &Credential) -> Result<(&'static str, &'static str, Value)> {
        match self {
            Self::FavoriteSong {
                add,
                song_id,
                song_type,
            } => {
                if song_id <= 0 || song_type < 0 {
                    return Err(QmError::ValueError("invalid favorite song identity".into()));
                }
                Ok((
                    "music.musicasset.PlaylistDetailWrite",
                    if add { "AddSonglist" } else { "DelSonglist" },
                    json!({"dirId": 201, "tid": 0, "bFmtUtf8": true,
                        "v_songInfo": [{"songId": song_id, "songType": song_type}]}),
                ))
            }
            Self::PlaylistTracks {
                add,
                dir_id,
                tid,
                songs,
            } => {
                if dir_id <= 0 || tid < 0 || songs.is_empty() || songs.len() > 100 {
                    return Err(QmError::ValueError(
                        "invalid playlist track mutation".into(),
                    ));
                }
                if songs
                    .iter()
                    .any(|(song_id, song_type)| *song_id <= 0 || *song_type < 0)
                {
                    return Err(QmError::ValueError("invalid playlist song identity".into()));
                }
                Ok((
                    "music.musicasset.PlaylistDetailWrite",
                    if add { "AddSonglist" } else { "DelSonglist" },
                    json!({"dirId": dir_id, "tid": tid, "bFmtUtf8": true,
                        "v_songInfo": songs.into_iter().map(|(song_id, song_type)|
                            json!({"songId": song_id, "songType": song_type})).collect::<Vec<_>>() }),
                ))
            }
            Self::CreatePlaylist { name } => {
                if name.is_empty() || name.chars().count() > 128 {
                    return Err(QmError::ValueError("invalid playlist name".into()));
                }
                Ok((
                    "music.musicasset.PlaylistBaseWrite",
                    "AddPlaylist",
                    json!({"dirName": name}),
                ))
            }
            Self::DeletePlaylist { dir_id } => {
                if dir_id <= 0 {
                    return Err(QmError::ValueError("invalid playlist id".into()));
                }
                Ok((
                    "music.musicasset.PlaylistBaseWrite",
                    "DelPlaylist",
                    json!({"dirId": dir_id}),
                ))
            }
            Self::EditPlaylist {
                dir_id,
                mask,
                name,
                description,
                picture_url,
                tag_list,
            } => {
                if dir_id <= 0 || mask <= 0 || name.is_empty() || name.chars().count() > 128 {
                    return Err(QmError::ValueError("invalid playlist edit".into()));
                }
                Ok((
                    "music.musicasset.PlaylistBaseWrite",
                    "EditPlaylist",
                    json!({
                        "dirId": dir_id, "mask": mask, "dirNewName": name,
                        "dirNewDesc": description, "dirNewPicUrl": picture_url,
                        "dirNewtaglist": tag_list
                    }),
                ))
            }
            Self::CollectPlaylist {
                collect,
                playlist_id,
            } => {
                if playlist_id <= 0 {
                    return Err(QmError::ValueError("invalid playlist id".into()));
                }
                if credential.encrypt_uin.is_empty() {
                    return Err(QmError::CredentialInvalid(
                        "playlist collection requires encrypted account identity".into(),
                    ));
                }
                Ok((
                    "music.musicasset.PlaylistFavWrite",
                    if collect {
                        "FavPlaylist"
                    } else {
                        "CancelFavPlaylist"
                    },
                    json!({"uin": credential.encrypt_uin, "v_playlistId": [playlist_id]}),
                ))
            }
        }
    }
}

fn account_write_comm(credential: &Credential) -> Value {
    let uin = credential.str_musicid();
    let gtk = crate::hash33(&credential.musickey, 5381);
    json!({
        "ct": "11", "cv": 13_020_508, "v": 13_020_508,
        "tmeAppID": "qqmusic", "format": "json", "inCharset": "utf-8",
        "outCharset": "utf-8", "notice": 0, "needNewCode": 1,
        "platform": "yqq.json", "uid": uin, "qq": uin, "uin": uin,
        "loginUin": uin, "authst": credential.musickey,
        "tmeLoginType": credential.login_type.to_string(), "g_tk": gtk,
        "g_tk_new_20200303": gtk
    })
}

fn cancelled() -> QmError {
    QmError::Network(crate::NetworkError {
        kind: crate::NetworkErrorKind::Cancelled,
        message: "account operation cancelled".into(),
    })
}

fn invalid(message: &'static str) -> QmError {
    QmError::ApiData(message.into())
}

fn check_code(value: &Value, required: bool) -> Result<()> {
    for key in ["code", "subcode"] {
        match value.get(key) {
            Some(raw) => {
                let code = raw
                    .as_i64()
                    .ok_or_else(|| invalid("invalid account response code"))?;
                if code != 0 {
                    return Err(crate::reply::map_cgi_code(code, &Value::Null));
                }
            }
            None if required && key == "code" => {
                return Err(invalid("missing account response code"))
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_page(
    envelope: Value,
    operation: &AccountRead,
    offset: u64,
    limit: u32,
) -> Result<AccountPage> {
    check_code(&envelope, true)?;
    let req = envelope
        .get("req")
        .or_else(|| envelope.get("req_0"))
        .ok_or_else(|| invalid("missing account response"))?;
    check_code(req, true)?;
    let data = req
        .get("data")
        .filter(|v| v.is_object())
        .ok_or_else(|| invalid("missing account page data"))?;
    check_code(data, false)?;
    for key in ["song_begin", "begin", "sin", "offset"] {
        if let Some(value) = data.get(key) {
            if value.as_u64() != Some(offset) {
                return Err(invalid("account page offset mismatch"));
            }
        }
    }
    let total = ["total_song_num", "total", "totalnum", "totalNum"]
        .into_iter()
        .find_map(|key| data.get(key))
        .map(|v| v.as_u64().ok_or_else(|| invalid("invalid page total")))
        .transpose()?;
    let rows = match operation {
        AccountRead::OwnedPlaylists => data.get("v_playlist").or_else(|| data.get("playlist")),
        AccountRead::CollectedPlaylists => data
            .get("v_list")
            .or_else(|| data.get("v_playlist"))
            .or_else(|| data.get("playlist")),
        AccountRead::RecentlyPlayed => data
            .get("songlist")
            .or_else(|| data.get("tracks"))
            .or_else(|| data.get("vecPlayRecord")),
        _ => data
            .get("songlist")
            .or_else(|| data.pointer("/cdlist/0/songlist"))
            .or_else(|| data.get("tracks")),
    }
    .and_then(Value::as_array)
    .ok_or_else(|| invalid("missing account row list"))?;
    if rows.len() > limit as usize {
        return Err(invalid("account page exceeds requested limit"));
    }
    let row_count = rows.len();
    let next = offset
        .checked_add(row_count as u64)
        .ok_or_else(|| invalid("account page overflow"))?;
    if total.is_some_and(|total| next > total && row_count > 0) {
        return Err(invalid("inconsistent account page total"));
    }
    let mut more = None;
    for (key, inverted) in [("hasmore", false), ("has_more", false), ("bFinish", true)] {
        if key == "bFinish"
            && !matches!(
                operation,
                AccountRead::OwnedPlaylists | AccountRead::CollectedPlaylists
            )
        {
            continue;
        }
        let Some(value) = data.get(key) else {
            continue;
        };
        let value = match value {
            Value::Bool(value) => *value,
            v if v.as_u64() == Some(0) => false,
            v if v.as_u64() == Some(1) => true,
            _ => return Err(invalid("invalid account continuation flag")),
        } ^ inverted;
        if more.is_some_and(|prior| prior != value) {
            return Err(invalid("conflicting account continuation flags"));
        }
        more = Some(value);
    }
    let has_more = more.unwrap_or_else(|| total.map_or(row_count > 0, |total| next < total));
    if row_count == 0 && has_more {
        return Err(invalid("account page made no progress"));
    }
    if more == Some(true) && total.is_some_and(|total| next >= total) {
        return Err(invalid("inconsistent account hasmore"));
    }
    if matches!(
        operation,
        AccountRead::OwnedPlaylists | AccountRead::CollectedPlaylists
    ) && more == Some(false)
        && total.is_some_and(|total| next < total)
    {
        return Err(invalid("account list finished before its declared total"));
    }
    if let AccountRead::PlaylistTracks { tid } | AccountRead::OwnedPlaylistTracks { tid, .. } =
        operation
    {
        let info = data
            .get("dirinfo")
            .or_else(|| data.pointer("/cdlist/0"))
            .ok_or_else(|| invalid("missing playlist identity"))?;
        let bound_dir = match operation {
            AccountRead::OwnedPlaylistTracks { dir_id, .. } => Some(*dir_id),
            _ => None,
        };
        if !playlist_identity_matches(info, tid, bound_dir) {
            return Err(invalid("playlist identity mismatch"));
        }
    }
    Ok(AccountPage {
        envelope,
        next_offset: has_more.then_some(next),
        total,
        row_count,
    })
}

fn playlist_identity_matches(info: &Value, tid: &str, bound_dir: Option<u64>) -> bool {
    let numeric_id = |value: &Value| value.as_u64().or_else(|| value.as_str()?.parse().ok());
    let same_tid = |value: &Value| {
        value.as_str() == Some(tid) || value.as_u64().is_some_and(|id| id.to_string() == tid)
    };
    let directories: Vec<_> = ["dirid", "dirId"]
        .into_iter()
        .filter_map(|key| info.get(key))
        .collect();
    if let Some(expected) = bound_dir {
        if expected == 0
            || directories
                .iter()
                .any(|value| numeric_id(value) != Some(expected))
        {
            return false;
        }
    }
    let identities: Vec<_> = ["tid", "disstid", "dissid"]
        .into_iter()
        .filter_map(|key| info.get(key))
        .collect();
    if !identities.is_empty() {
        // A conflicting explicit TID cannot be hidden by a matching directory.
        return identities.into_iter().all(same_tid);
    }
    if info.get("id").is_some_and(same_tid) {
        return true;
    }
    bound_dir.is_some_and(|expected| {
        !directories.is_empty() && info.get("id").and_then(numeric_id) == Some(expected)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(data: Value) -> Value {
        json!({"code": 0, "req": {"code": 0, "data": data}})
    }

    #[test]
    fn parse_favorite_page_preserves_rows_and_cursor() {
        let value = envelope(json!({
            "song_begin": 0,
            "total_song_num": 3,
            "hasmore": 1,
            "songlist": [{"mid": "a"}, {"mid": "b"}]
        }));
        let page = parse_page(value, &AccountRead::FavoriteSongs, 0, 2).unwrap();
        assert_eq!(page.row_count, 2);
        assert_eq!(page.total, Some(3));
        assert_eq!(page.next_offset, Some(2));
    }

    #[test]
    fn parse_playlist_page_rejects_foreign_identity() {
        let value = envelope(json!({
            "total": 1,
            "songlist": [{"mid": "a"}],
            "dirinfo": {"tid": "other"}
        }));
        let result = parse_page(
            value,
            &AccountRead::PlaylistTracks {
                tid: "expected".into(),
            },
            0,
            10,
        );
        assert!(
            matches!(result, Err(QmError::ApiData(message)) if message == "playlist identity mismatch")
        );
    }

    #[test]
    fn owned_playlist_directory_requires_an_explicit_matching_binding() {
        let page = envelope(json!({
            "total": 1, "songlist": [{"mid":"a"}], "dirinfo":{"id":3001,"dirid":3001}
        }));
        let operation = AccountRead::OwnedPlaylistTracks {
            tid: "playlist-tid".into(),
            dir_id: 3001,
        };
        assert_eq!(
            parse_page(page.clone(), &operation, 0, 10)
                .unwrap()
                .row_count,
            1
        );
        assert!(parse_page(
            page.clone(),
            &AccountRead::PlaylistTracks {
                tid: "playlist-tid".into()
            },
            0,
            10
        )
        .is_err());
        assert!(parse_page(
            page,
            &AccountRead::OwnedPlaylistTracks {
                tid: "playlist-tid".into(),
                dir_id: 3002
            },
            0,
            10
        )
        .is_err());
        for info in [
            json!({"id":3001}),
            json!({"id":3001,"dirid":3002}),
            json!({"id":3001,"dirid":3001,"dirId":3002}),
            json!({"id":3001,"dirid":3001,"tid":"another-playlist"}),
            json!({"id":3001,"dirid":3001,"tid":"playlist-tid","disstid":"another-playlist"}),
            json!({"id":3001,"dirid":3001,"tid":null}),
        ] {
            assert!(parse_page(
                envelope(json!({"total":1,"songlist":[{}],"dirinfo":info})),
                &operation,
                0,
                10
            )
            .is_err());
        }
        assert!(parse_page(
            envelope(json!({
                "total":1,"songlist":[{}],"dirinfo":{"tid":"playlist-tid","dirId":3001}
            })),
            &operation,
            0,
            10
        )
        .is_ok());
    }

    #[test]
    fn parse_page_rejects_empty_page_claiming_more() {
        let value = envelope(json!({
            "total": 1,
            "hasmore": true,
            "songlist": []
        }));
        let result = parse_page(value, &AccountRead::FavoriteSongs, 0, 10);
        assert!(
            matches!(result, Err(QmError::ApiData(message)) if message == "account page made no progress")
        );
    }

    #[tokio::test]
    async fn private_executor_rejects_unknown_endpoint_before_transport() {
        let client = Client::new(None, None).unwrap();
        let credential = Credential {
            musicid: 1,
            musickey: "key".into(),
            ..Credential::default()
        };
        let result = execute_write(
            &client,
            &credential,
            "music.unknown.Module",
            "Unknown",
            json!({}),
            CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(result, Err(QmError::ValueError(message)) if message == "unsupported account write endpoint")
        );
    }

    #[test]
    fn typed_write_builds_fixed_favorite_endpoint() {
        let (module, method, param) = AccountWrite::FavoriteSong {
            add: true,
            song_id: 42,
            song_type: 0,
        }
        .into_wire(&Credential::default())
        .unwrap();
        assert_eq!(module, "music.musicasset.PlaylistDetailWrite");
        assert_eq!(method, "AddSonglist");
        assert_eq!(param["dirId"], 201);
        assert_eq!(param["v_songInfo"][0]["songId"], 42);
    }

    #[test]
    fn typed_write_rejects_empty_playlist_mutation() {
        let result = AccountWrite::PlaylistTracks {
            add: true,
            dir_id: 1,
            tid: 0,
            songs: Vec::new(),
        }
        .into_wire(&Credential::default());
        assert!(
            matches!(result, Err(QmError::ValueError(message)) if message == "invalid playlist track mutation")
        );
    }
}
