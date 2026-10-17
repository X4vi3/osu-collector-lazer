//! osucollector.com API client.
//!
//! `beatmapsV3` returns the whole collection in one response (beatmaps + beatmapsets);
//! `beatmapsV2` is cursor-paginated (max 100 per page) and kept as a fallback.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BASE: &str = "https://osucollector.com/api/collections";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionInfo {
    pub id: u64,
    pub name: String,
    pub uploader: String,
    pub beatmap_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Beatmap {
    pub id: u64,
    pub set_id: u64,
    pub checksum: String,
    pub version: String,
    /// osu!standard only after normalisation; other modes are dropped on load.
    pub sr: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BeatmapSet {
    pub id: u64,
    pub artist: String,
    pub title: String,
    pub creator: String,
    pub cover: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionData {
    pub info: CollectionInfo,
    /// osu!standard difficulties only
    pub beatmaps: Vec<Beatmap>,
    pub sets: Vec<BeatmapSet>,
    /// Unix seconds
    pub fetched_at: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(
        "{}",
        crate::core::i18n::tr(
            "Коллекция не найдена или приватная",
            "Collection not found or private"
        )
    )]
    NotFound,
    #[error("{p}: {0}", p = crate::core::i18n::tr("Не удалось связаться с osucollector.com", "Could not reach osucollector.com"))]
    Network(String),
    #[error("{p} {0}", p = crate::core::i18n::tr("osucollector.com ответил ошибкой", "osucollector.com returned error"))]
    Status(u16),
    #[error("{p}: {0}", p = crate::core::i18n::tr("Неожиданный ответ osucollector.com", "Unexpected response from osucollector.com"))]
    Parse(String),
}

impl From<reqwest::Error> for ApiError {
    fn from(e: reqwest::Error) -> Self {
        if let Some(s) = e.status() {
            if s.as_u16() == 404 {
                return ApiError::NotFound;
            }
            return ApiError::Status(s.as_u16());
        }
        if e.is_decode() {
            return ApiError::Parse(e.to_string());
        }
        ApiError::Network(e.to_string())
    }
}

/// Accepts `https://osucollector.com/collections/123/slug`, `osucollector.com/collections/123`, or `123`.
pub fn parse_collection_id(input: &str) -> Option<u64> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return s.parse().ok().filter(|&n| n > 0);
    }
    let lower = s.to_ascii_lowercase();
    if !lower.contains("osucollector.com") {
        return None;
    }
    let idx = lower.find("/collections/")?;
    let rest = &s[idx + "/collections/".len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok().filter(|&n| n > 0)
}

pub fn is_std_mode(mode: &Value) -> bool {
    match mode {
        Value::String(s) => s == "osu",
        Value::Number(n) => n.as_u64() == Some(0),
        _ => false,
    }
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn parse_beatmap(v: &Value) -> Option<Beatmap> {
    if !is_std_mode(v.get("mode")?) {
        return None;
    }
    let set_id = v
        .get("beatmapset_id")
        .and_then(Value::as_u64)
        .or_else(|| v.get("beatmapset")?.get("id")?.as_u64())?;
    let checksum = v.get("checksum")?.as_str()?.to_ascii_lowercase();
    if checksum.len() != 32 {
        return None;
    }
    Some(Beatmap {
        id: v.get("id").and_then(Value::as_u64).unwrap_or(0),
        set_id,
        checksum,
        version: str_field(v, "version"),
        sr: v
            .get("difficulty_rating")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
    })
}

fn parse_set(v: &Value) -> Option<BeatmapSet> {
    let cover = v
        .get("cover")
        .and_then(Value::as_str)
        .or_else(|| v.get("covers")?.get("card")?.as_str())
        .map(str::to_string);
    Some(BeatmapSet {
        id: v.get("id")?.as_u64()?,
        artist: str_field(v, "artist"),
        title: str_field(v, "title"),
        creator: str_field(v, "creator"),
        cover,
    })
}

/// Parses a `beatmapsV3` body: `{ beatmaps: [...], beatmapsets: [...] }`.
pub fn parse_v3(body: &Value) -> Result<(Vec<Beatmap>, Vec<BeatmapSet>), ApiError> {
    let maps = body
        .get("beatmaps")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Parse("no beatmaps field".into()))?;
    let sets = body
        .get("beatmapsets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let beatmaps: Vec<Beatmap> = maps.iter().filter_map(parse_beatmap).collect();
    let sets = sets.iter().filter_map(parse_set).collect();
    Ok((beatmaps, sets))
}

/// Parses one `beatmapsV2` page; set metadata is embedded in each beatmap.
pub fn parse_v2_page(
    body: &Value,
) -> Result<(Vec<Beatmap>, Vec<BeatmapSet>, Option<u64>), ApiError> {
    let maps = body
        .get("beatmaps")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Parse("no beatmaps field".into()))?;
    let beatmaps = maps.iter().filter_map(parse_beatmap).collect();
    let sets = maps
        .iter()
        .filter_map(|m| parse_set(m.get("beatmapset")?))
        .collect();
    let has_more = body
        .get("hasMore")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let cursor = if has_more {
        body.get("nextPageCursor").and_then(Value::as_u64)
    } else {
        None
    };
    Ok((beatmaps, sets, cursor))
}

pub fn parse_info(body: &Value) -> Result<CollectionInfo, ApiError> {
    Ok(CollectionInfo {
        id: body
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| ApiError::Parse("no id".into()))?,
        name: str_field(body, "name"),
        uploader: body
            .get("uploader")
            .and_then(|u| u.get("username"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        beatmap_count: body
            .get("beatmapCount")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value, ApiError> {
    let resp = client.get(url).send().await?;
    if resp.status().as_u16() == 404 {
        return Err(ApiError::NotFound);
    }
    let resp = resp.error_for_status()?;
    Ok(resp.json::<Value>().await?)
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "stage", rename_all = "camelCase")]
pub enum LoadProgress {
    Info,
    Beatmaps { loaded: u64, total: u64 },
}

pub async fn fetch_collection(
    client: &reqwest::Client,
    id: u64,
    mut progress: impl FnMut(LoadProgress),
) -> Result<CollectionData, ApiError> {
    progress(LoadProgress::Info);
    let info = parse_info(&get_json(client, &format!("{BASE}/{id}")).await?)?;
    progress(LoadProgress::Beatmaps {
        loaded: 0,
        total: info.beatmap_count,
    });

    let (beatmaps, sets) = match get_json(client, &format!("{BASE}/{id}/beatmapsV3"))
        .await
        .and_then(|b| parse_v3(&b))
    {
        Ok(r) => r,
        Err(ApiError::NotFound) => return Err(ApiError::NotFound),
        Err(_) => fetch_v2(client, id, info.beatmap_count, &mut progress).await?,
    };
    progress(LoadProgress::Beatmaps {
        loaded: info.beatmap_count,
        total: info.beatmap_count,
    });

    let mut sets = sets;
    sets.sort_by_key(|s| s.id);
    sets.dedup_by_key(|s| s.id);
    let mut beatmaps = beatmaps;
    // A diff may appear twice in a collection; keep one per checksum.
    let mut seen = std::collections::HashSet::new();
    beatmaps.retain(|b| seen.insert(b.checksum.clone()));

    Ok(CollectionData {
        info,
        beatmaps,
        sets,
        fetched_at: now_secs(),
    })
}

async fn fetch_v2(
    client: &reqwest::Client,
    id: u64,
    total: u64,
    progress: &mut impl FnMut(LoadProgress),
) -> Result<(Vec<Beatmap>, Vec<BeatmapSet>), ApiError> {
    let mut beatmaps = Vec::new();
    let mut sets = Vec::new();
    let mut cursor: Option<u64> = None;
    let mut fetched = 0u64;
    loop {
        let mut url = format!("{BASE}/{id}/beatmapsV2?perPage=100");
        if let Some(c) = cursor {
            url.push_str(&format!("&cursor={c}"));
        }
        let body = get_json(client, &url).await?;
        let page_len = body
            .get("beatmaps")
            .and_then(Value::as_array)
            .map_or(0, |a| a.len()) as u64;
        let (b, s, next) = parse_v2_page(&body)?;
        beatmaps.extend(b);
        sets.extend(s);
        fetched += page_len;
        progress(LoadProgress::Beatmaps {
            loaded: fetched,
            total,
        });
        match next {
            Some(c) if page_len > 0 => cursor = Some(c),
            _ => break,
        }
    }
    Ok((beatmaps, sets))
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_links_and_ids() {
        assert_eq!(parse_collection_id("12345"), Some(12345));
        assert_eq!(parse_collection_id("  44 "), Some(44));
        assert_eq!(
            parse_collection_id("https://osucollector.com/collections/12345/some-slug"),
            Some(12345)
        );
        assert_eq!(
            parse_collection_id("https://osucollector.com/collections/12345"),
            Some(12345)
        );
        assert_eq!(
            parse_collection_id("osucollector.com/collections/777?x=1"),
            Some(777)
        );
        assert_eq!(
            parse_collection_id("http://www.osucollector.com/collections/9/"),
            Some(9)
        );
        assert_eq!(
            parse_collection_id("https://OSUCOLLECTOR.com/Collections/31"),
            Some(31)
        );
        assert_eq!(parse_collection_id(""), None);
        assert_eq!(parse_collection_id("0"), None);
        assert_eq!(parse_collection_id("abc"), None);
        assert_eq!(
            parse_collection_id("https://example.com/collections/5"),
            None
        );
        assert_eq!(
            parse_collection_id("https://osucollector.com/collections/"),
            None
        );
    }

    #[test]
    fn parses_v3_and_drops_other_modes() {
        let body = json!({
            "beatmaps": [
                {"id": 1, "beatmapset_id": 10, "checksum": "C3F3D6BA82CE48DFE2BFB049A184582C", "version": "Hard", "mode": "osu", "difficulty_rating": 7.2},
                {"id": 2, "beatmapset_id": 10, "checksum": "8d2c3b7dfe5192649ac84a2febc7b8b0", "version": "Taiko", "mode": "taiko", "difficulty_rating": 3.0},
                {"id": 3, "beatmapset_id": 11, "checksum": "8d2c3b7dfe5192649ac84a2febc7b8b1", "version": "X", "mode": 0, "difficulty_rating": null},
                {"id": 4, "beatmapset_id": 11, "checksum": "short", "mode": "osu"}
            ],
            "beatmapsets": [
                {"id": 10, "artist": "A", "title": "T", "creator": "C", "cover": "https://x/c.jpg"}
            ]
        });
        let (maps, sets) = parse_v3(&body).unwrap();
        assert_eq!(maps.len(), 2);
        assert_eq!(maps[0].checksum, "c3f3d6ba82ce48dfe2bfb049a184582c");
        assert_eq!(maps[0].sr, 7.2);
        assert_eq!(maps[1].sr, 0.0);
        assert_eq!(sets[0].cover.as_deref(), Some("https://x/c.jpg"));
    }

    #[test]
    fn parses_v2_page_cursor() {
        let body = json!({
            "nextPageCursor": 555, "hasMore": true,
            "beatmaps": [{"id": 1, "beatmapset_id": 10, "checksum": "c3f3d6ba82ce48dfe2bfb049a184582c", "version": "Hard", "mode": "osu", "difficulty_rating": 7.2,
                "beatmapset": {"id": 10, "artist": "A", "title": "T", "creator": "C", "covers": {"card": "https://x/card.jpg"}}}]
        });
        let (maps, sets, cursor) = parse_v2_page(&body).unwrap();
        assert_eq!(maps.len(), 1);
        assert_eq!(sets[0].cover.as_deref(), Some("https://x/card.jpg"));
        assert_eq!(cursor, Some(555));
        let last = json!({"nextPageCursor": null, "hasMore": false, "beatmaps": []});
        assert_eq!(parse_v2_page(&last).unwrap().2, None);
    }

    /// Live check against the real API; run with `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_fetch_small_collection() {
        let client = crate::core::http_client();
        let data = fetch_collection(&client, 44, |_| {}).await.unwrap();
        assert_eq!(data.info.name, "speed practice");
        assert!(data.beatmaps.len() > 500);
        assert!(data.sets.len() > 100);
        let err = fetch_collection(&client, 1, |_| {}).await.unwrap_err();
        assert!(matches!(err, ApiError::NotFound));
    }
}
