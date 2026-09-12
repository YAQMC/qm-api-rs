//! Account-scoped reads used by desktop and mobile hosts.
//!
//! Endpoint and credential selection live here. `AccountPage` keeps a bounded
//! compatibility envelope while hosts migrate their legacy song/playlist DTOs.
//! Offsets count upstream rows, not successfully rendered songs.

use serde_json::{json, Value};

use crate::{CancellationToken, Client, Credential, HttpMethod, HttpOptions, QmError, Result};

/// The only account read operations exposed by this compatibility boundary.
#[derive(Clone, Debug)]
pub enum AccountRead {
    FavoriteSongs,
    PlaylistTracks { tid: String },
    RecentlyPlayed,
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
        AccountRead::FavoriteSongs | AccountRead::PlaylistTracks { .. } => {
            let mut param = json!({
                "disstid": 0, "dirid": 201, "tag": true,
                "song_begin": offset, "song_num": limit,
                "userinfo": true, "orderlist": true, "onlysonglist": 1
            });
            match &operation {
                AccountRead::PlaylistTracks { tid } => {
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

fn cancelled() -> QmError {
    QmError::Network(crate::NetworkError {
        kind: crate::NetworkErrorKind::Cancelled,
        message: "account read cancelled".into(),
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
    .ok_or_else(|| invalid("missing account song list"))?;
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
    let more = match data.get("hasmore").or_else(|| data.get("has_more")) {
        None => None,
        Some(Value::Bool(more)) => Some(*more),
        Some(v) if v.as_u64() == Some(0) => Some(false),
        Some(v) if v.as_u64() == Some(1) => Some(true),
        _ => return Err(invalid("invalid account hasmore")),
    };
    let has_more = more.unwrap_or_else(|| total.map_or(row_count > 0, |total| next < total));
    if row_count == 0 && has_more {
        return Err(invalid("account page made no progress"));
    }
    if more == Some(true) && total.is_some_and(|total| next >= total) {
        return Err(invalid("inconsistent account hasmore"));
    }
    if let AccountRead::PlaylistTracks { tid } = operation {
        let info = data
            .get("dirinfo")
            .or_else(|| data.pointer("/cdlist/0"))
            .ok_or_else(|| invalid("missing playlist identity"))?;
        let actual = ["tid", "disstid", "dissid", "id"]
            .into_iter()
            .find_map(|key| info.get(key));
        let actual = actual.and_then(|v| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        });
        if actual.as_deref() != Some(tid.as_str()) {
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
}
