//! Resolving beatmap MD5s (from a `collection.db`) to star rating and beatmapset via mirror APIs.
//!
//! catboy `/api/v2/md5/{hash}` returns the beatmap with its set metadata; osu.direct
//! `/api/v2/md5/{hash}` is the fallback (no artist/title). Results, including misses, are cached.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::osucollector::{Beatmap, BeatmapSet, is_std_mode};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Found {
    pub beatmap: Beatmap,
    pub std: bool,
    pub set: Option<BeatmapSet>,
}

/// md5 → result; `None` = no mirror knows this hash.
pub type Md5Cache = HashMap<String, Option<Found>>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupProgress {
    pub done: u32,
    pub total: u32,
    pub found: u32,
    pub eta_secs: Option<u64>,
}

const CONCURRENCY: usize = 6;
/// Total request rate across workers; catboy allows 1200/min.
const MIN_INTERVAL: Duration = Duration::from_millis(85);

pub fn parse_lookup(v: &Value) -> Option<Found> {
    let checksum = v.get("checksum")?.as_str()?.to_ascii_lowercase();
    let set_id = v.get("beatmapset_id").and_then(Value::as_u64)?;
    let std = v.get("mode").is_some_and(is_std_mode)
        || v.get("mode_int").and_then(Value::as_u64) == Some(0);
    let set = v.get("set").and_then(|s| {
        Some(BeatmapSet {
            id: s.get("id")?.as_u64()?,
            artist: s
                .get("artist")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            title: s
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            creator: s
                .get("creator")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            cover: s
                .get("covers")
                .and_then(|c| c.get("card"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    Some(format!(
                        "https://assets.ppy.sh/beatmaps/{set_id}/covers/card.jpg"
                    ))
                }),
        })
    });
    Some(Found {
        beatmap: Beatmap {
            id: v.get("id").and_then(Value::as_u64).unwrap_or(0),
            set_id,
            checksum,
            version: v
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            sr: v
                .get("difficulty_rating")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
        },
        std,
        set,
    })
}

enum One {
    Found(Found),
    Missing,
    /// transient: rate limit / network / server error
    Retry,
}

async fn query(client: &reqwest::Client, url: String) -> One {
    match client.get(url).send().await {
        Ok(r) if r.status().as_u16() == 404 => One::Missing,
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(v) => parse_lookup(&v).map_or(One::Missing, One::Found),
            Err(_) => One::Retry,
        },
        _ => One::Retry,
    }
}

async fn lookup_one(
    client: &reqwest::Client,
    md5: &str,
    cancel: &CancellationToken,
) -> Option<Option<Found>> {
    for attempt in 0..4u32 {
        let catboy_missing =
            match query(client, format!("https://catboy.best/api/v2/md5/{md5}")).await {
                One::Found(f) => return Some(Some(f)),
                One::Missing => true,
                One::Retry => false,
            };
        // catboy didn't know it or is limiting us: ask osu.direct
        match query(client, format!("https://osu.direct/api/v2/md5/{md5}")).await {
            One::Found(f) => return Some(Some(f)),
            One::Missing if catboy_missing => return Some(None),
            _ => {}
        }
        tokio::select! {
            _ = cancel.cancelled() => return None,
            _ = tokio::time::sleep(Duration::from_secs(2 << attempt)) => {}
        }
    }
    None
}

/// Looks up every hash not already in `cache`, updating it as results arrive.
/// Returns when done or cancelled; unresolved hashes simply stay out of the cache.
pub async fn lookup_all(
    client: &reqwest::Client,
    hashes: &[String],
    cache: Arc<Mutex<Md5Cache>>,
    cancel: CancellationToken,
    progress: impl Fn(LookupProgress) + Send + Sync + 'static,
) {
    let todo: Vec<String> = {
        let c = cache.lock().unwrap();
        hashes
            .iter()
            .filter(|h| !c.contains_key(*h))
            .cloned()
            .collect()
    };
    let total = hashes.len() as u32;
    let already = total - todo.len() as u32;
    let found_cached = {
        let c = cache.lock().unwrap();
        hashes
            .iter()
            .filter(|h| matches!(c.get(*h), Some(Some(_))))
            .count() as u32
    };
    let state = Arc::new(Mutex::new((already, found_cached)));
    let progress = Arc::new(progress);
    progress(LookupProgress {
        done: already,
        total,
        found: found_cached,
        eta_secs: None,
    });

    let started = Instant::now();
    let sem = Arc::new(Semaphore::new(CONCURRENCY));
    let pacer = Arc::new(tokio::sync::Mutex::new(Instant::now()));
    let mut handles = Vec::new();
    for md5 in todo {
        let permit = tokio::select! {
            _ = cancel.cancelled() => break,
            p = sem.clone().acquire_owned() => p.unwrap(),
        };
        {
            let mut next = pacer.lock().await;
            let now = Instant::now();
            if *next > now {
                tokio::time::sleep(*next - now).await;
            }
            *next = Instant::now() + MIN_INTERVAL;
        }
        let (client, cache, cancel, state, progress) = (
            client.clone(),
            cache.clone(),
            cancel.clone(),
            state.clone(),
            progress.clone(),
        );
        handles.push(tokio::spawn(async move {
            let r = lookup_one(&client, &md5, &cancel).await;
            drop(permit);
            let Some(r) = r else { return };
            let hit = r.is_some();
            cache.lock().unwrap().insert(md5, r);
            let (done, found) = {
                let mut s = state.lock().unwrap();
                s.0 += 1;
                if hit {
                    s.1 += 1;
                }
                *s
            };
            let fresh = done - already;
            let rate = fresh as f64 / started.elapsed().as_secs_f64().max(0.5);
            let eta = (rate > 0.0 && fresh >= 10).then(|| ((total - done) as f64 / rate) as u64);
            progress(LookupProgress {
                done,
                total,
                found,
                eta_secs: eta,
            });
        }));
    }
    for h in handles {
        let _ = h.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_catboy_and_osudirect_shapes() {
        let catboy = json!({"id": 1942387, "beatmapset_id": 930062, "checksum": "C3F3D6BA82CE48DFE2BFB049A184582C",
            "mode": "osu", "mode_int": 0, "version": "Extreme", "difficulty_rating": 6.82,
            "set": {"id": 930062, "artist": "Yuyoyuppe", "title": "-ELIS-", "creator": "Seto Kousuke", "covers": {"card": "https://x/card.jpg"}}});
        let f = parse_lookup(&catboy).unwrap();
        assert!(f.std);
        assert_eq!(f.beatmap.checksum, "c3f3d6ba82ce48dfe2bfb049a184582c");
        assert_eq!(f.set.unwrap().title, "-ELIS-");

        let direct = json!({"id": 5, "beatmapset_id": 6, "checksum": "8d2c3b7dfe5192649ac84a2febc7b8b0", "mode": "mania", "mode_int": 3, "version": "4K", "difficulty_rating": 3.1});
        let f = parse_lookup(&direct).unwrap();
        assert!(!f.std);
        assert!(f.set.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn live_lookup() {
        let cache = Arc::new(Mutex::new(Md5Cache::new()));
        let hashes = vec![
            "c3f3d6ba82ce48dfe2bfb049a184582c".to_string(),
            "ffffffffffffffffffffffffffffffff".to_string(),
        ];
        lookup_all(
            &crate::core::http_client(),
            &hashes,
            cache.clone(),
            CancellationToken::new(),
            |_| {},
        )
        .await;
        let c = cache.lock().unwrap();
        assert_eq!(c[&hashes[0]].as_ref().unwrap().beatmap.set_id, 930062);
        assert!(c[&hashes[1]].is_none());
    }
}
