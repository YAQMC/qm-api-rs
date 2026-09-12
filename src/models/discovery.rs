//! Typed cards shared by the web discovery endpoints. Unknown card kinds are
//! retained as metadata; callers need not interpret upstream nesting or URLs.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedCardKind {
    Playlist,
    NewSongs,
    Songlist,
    Artist,
    Other,
}

#[derive(Debug, Clone)]
pub struct FeedCard {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub cover_url: String,
    pub kind: FeedCardKind,
}

impl<'de> Deserialize<'de> for FeedCard {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(de)?;
        if !raw.is_object() {
            return Err(serde::de::Error::custom("feed card must be an object"));
        }
        let id = scalar_id(&raw["id"]);
        if id.trim().is_empty() {
            return Err(serde::de::Error::custom("feed card requires an identifier"));
        }
        let title = raw["title"]
            .as_str()
            .ok_or_else(|| serde::de::Error::custom("feed card requires a text title"))?;
        let kind = match (raw["type"].as_i64(), raw["subtype"].as_i64()) {
            (Some(500), Some(511)) => FeedCardKind::NewSongs,
            (Some(500), _) => FeedCardKind::Playlist,
            (Some(700), _) => FeedCardKind::Songlist,
            (Some(600), _) => FeedCardKind::Artist,
            _ => FeedCardKind::Other,
        };
        Ok(Self {
            id,
            title: title.to_owned(),
            subtitle: text(&raw["subtitle"]),
            cover_url: card_cover(&raw),
            kind,
        })
    }
}

#[derive(Debug, Clone)]
pub struct FeedShelf {
    pub id: Option<i64>,
    pub cards: Vec<FeedCard>,
}

impl<'de> Deserialize<'de> for FeedShelf {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Niche {
            v_card: Vec<FeedCard>,
        }
        #[derive(Deserialize)]
        struct Shelf {
            #[serde(default)]
            id: Option<i64>,
            v_niche: Vec<Niche>,
        }
        let raw = Shelf::deserialize(de)?;
        Ok(Self {
            id: raw.id,
            cards: raw
                .v_niche
                .into_iter()
                .flat_map(|niche| niche.v_card)
                .collect(),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AreaPage {
    #[serde(default)]
    pub title: String,
    #[serde(rename = "v_shelf")]
    pub shelves: Vec<FeedShelf>,
}

#[derive(Debug, Clone)]
pub struct CategoryCard {
    pub area_key: String,
    pub title: String,
    pub cover_url: String,
}

#[derive(Debug, Clone)]
pub struct NewMvCard {
    pub id: String,
    pub title: String,
    pub cover_url: String,
    pub duration_seconds: u64,
    pub artist: String,
}

impl<'de> Deserialize<'de> for NewMvCard {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(de)?;
        let id = raw["mvid"]
            .as_u64()
            .filter(|id| *id > 0)
            .ok_or_else(|| serde::de::Error::custom("MV requires a positive identifier"))?;
        let title = raw["title"]
            .as_str()
            .ok_or_else(|| serde::de::Error::custom("MV requires a text title"))?;
        let duration_seconds = match raw.get("duration") {
            None | Some(Value::Null) => 0,
            Some(value) => value
                .as_u64()
                .ok_or_else(|| serde::de::Error::custom("invalid MV duration"))?,
        };
        Ok(Self {
            id: id.to_string(),
            title: title.to_owned(),
            cover_url: card_cover(&raw),
            duration_seconds,
            artist: raw["singers"]
                .as_array()
                .and_then(|singers| singers.first())
                .map(|singer| text(&singer["name"]))
                .unwrap_or_default(),
        })
    }
}

pub(crate) fn scalar_id(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) if value.is_u64() || value.is_i64() => value.to_string(),
        _ => String::new(),
    }
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// Decode known artwork fields. This is not a download authorization: the
/// host must still enforce its own origin and redirect policy before fetching.
pub(crate) fn card_cover(raw: &Value) -> String {
    let cover = &raw["cover"];
    let direct = cover
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let nested = [
        "url",
        "medium_url",
        "big_url",
        "default_url",
        "small_url",
        "PhotoUrl",
        "photo_url",
    ]
    .into_iter()
    .find_map(|key| {
        cover[key]
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    });
    direct
        .or(nested)
        .or_else(|| raw["picurl"].as_str())
        .unwrap_or_default()
        .to_owned()
}
