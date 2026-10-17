//! Read-only scan of lazer's file store to learn which difficulties are already installed.
//!
//! lazer keeps every file at `<data>/files/<h>/<hh>/<sha256>`. `.osu` files are recognised by
//! their header; for each we record its MD5 and the `BeatmapSetID` from `[Metadata]`.
//! `client.realm` is never opened. File names are content hashes, so results cache forever.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::trim::md5_hex;

/// `<APPDATA>/osu`, or the custom location from its `storage.ini` (`FullPath = …`).
pub fn data_dir() -> Option<PathBuf> {
    let base = PathBuf::from(std::env::var_os("APPDATA")?).join("osu");
    if let Ok(ini) = std::fs::read_to_string(base.join("storage.ini")) {
        for line in ini.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let v = v.trim();
                if k.trim().eq_ignore_ascii_case("FullPath")
                    && !v.is_empty()
                    && Path::new(v).join("files").is_dir()
                {
                    return Some(PathBuf::from(v));
                }
            }
        }
    }
    base.join("files").is_dir().then_some(base)
}

/// Per stored file: `Some((set_id, md5))` for a `.osu`, `None` for anything else.
pub type ScanCache = HashMap<String, Option<(i64, String)>>;

#[derive(Debug, Default, Clone)]
pub struct Installed {
    /// beatmapset id → MD5s of its installed difficulties
    pub by_set: HashMap<u64, HashSet<String>>,
    /// every installed difficulty, including ones without an online set id
    pub all: HashSet<String>,
}

impl Installed {
    pub fn set(&self, set_id: u64) -> Option<&HashSet<String>> {
        self.by_set.get(&set_id)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub found: bool,
    pub path: Option<String>,
    pub sets: u32,
    pub diffs: u32,
}

const OSU_MAGIC: &[u8] = b"osu file format v";

/// `BeatmapSetID:` from the `[Metadata]` section; -1/missing → `None`.
pub fn parse_set_id(text: &str) -> Option<i64> {
    let mut in_meta = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            if in_meta {
                return None;
            }
            in_meta = l.eq_ignore_ascii_case("[Metadata]");
            continue;
        }
        if in_meta {
            if let Some(v) = l.strip_prefix("BeatmapSetID:") {
                return v.trim().parse::<i64>().ok().filter(|&n| n > 0);
            }
        }
    }
    None
}

fn inspect(path: &Path) -> Option<(i64, String)> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 20];
    let n = f.read(&mut head).ok()?;
    let head = &head[..n];
    let body = head
        .strip_prefix(b"\xEF\xBB\xBF".as_slice())
        .unwrap_or(head);
    if !body.starts_with(OSU_MAGIC) {
        return None;
    }
    let mut bytes = head.to_vec();
    f.take(8 << 20).read_to_end(&mut bytes).ok()?;
    let set = parse_set_id(&String::from_utf8_lossy(&bytes)).unwrap_or(-1);
    Some((set, md5_hex(&bytes)))
}

fn list_files(files_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(l1) = std::fs::read_dir(files_dir) else {
        return out;
    };
    for a in l1.flatten() {
        let Ok(l2) = std::fs::read_dir(a.path()) else {
            continue;
        };
        for b in l2.flatten() {
            let Ok(l3) = std::fs::read_dir(b.path()) else {
                continue;
            };
            for c in l3.flatten() {
                if c.file_type().is_ok_and(|t| t.is_file()) {
                    out.push((c.file_name().to_string_lossy().into_owned(), c.path()));
                }
            }
        }
    }
    out
}

/// Scans the store, reusing `cache` for files seen before. Returns the index and the updated cache.
pub fn scan(data: &Path, mut cache: ScanCache) -> (Installed, ScanCache) {
    let files = list_files(&data.join("files"));
    let todo: Vec<&(String, PathBuf)> = files
        .iter()
        .filter(|(name, _)| !cache.contains_key(name))
        .collect();

    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(2, 8);
    let chunk = todo.len().div_ceil(threads).max(1);
    let fresh: Vec<(String, Option<(i64, String)>)> = std::thread::scope(|s| {
        let handles: Vec<_> = todo
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|(name, p)| (name.clone(), inspect(p)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    cache.extend(fresh);

    // forget files lazer has cleaned up
    let present: HashSet<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    cache.retain(|k, _| present.contains(k.as_str()));

    let mut idx = Installed::default();
    for (set, md5) in cache.values().flatten() {
        idx.all.insert(md5.clone());
        if *set > 0 {
            idx.by_set
                .entry(*set as u64)
                .or_default()
                .insert(md5.clone());
        }
    }
    (idx, cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OSU: &str = "osu file format v14\r\n\r\n[General]\r\nAudioFilename: a.mp3\r\n\r\n[Metadata]\r\nTitle:x\r\nBeatmapID:5\r\nBeatmapSetID:930062\r\n\r\n[Difficulty]\r\nHPDrainRate:5\r\n";

    #[test]
    fn parses_set_id_only_from_metadata() {
        assert_eq!(parse_set_id(OSU), Some(930062));
        assert_eq!(
            parse_set_id("osu file format v14\n[Metadata]\nBeatmapSetID:-1\n"),
            None
        );
        assert_eq!(
            parse_set_id(
                "osu file format v14\n[General]\nBeatmapSetID:5\n[Metadata]\nTitle:x\n[Events]\n"
            ),
            None
        );
    }

    #[test]
    fn scans_store_layout_and_uses_cache() {
        let dir = tempfile::tempdir().unwrap();
        let put = |name: &str, data: &[u8]| {
            let p = dir.path().join("files").join(&name[..1]).join(&name[..2]);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join(name), data).unwrap();
        };
        put("aa11", OSU.as_bytes());
        put(
            "ab22",
            format!("\u{feff}{OSU}").replace("930062", "7").as_bytes(),
        );
        put("cc33", b"ID3 not a beatmap");

        let (idx, cache) = scan(dir.path(), ScanCache::new());
        assert_eq!(idx.all.len(), 2);
        assert_eq!(
            idx.set(930062).unwrap().iter().next().unwrap(),
            &md5_hex(OSU.as_bytes())
        );
        assert!(idx.set(7).is_some());
        assert_eq!(cache.len(), 3);
        assert_eq!(cache["cc33"], None);

        // removed files drop out of the cache; unchanged ones are not re-read
        std::fs::remove_file(dir.path().join("files/a/aa/aa11")).unwrap();
        let (idx2, cache2) = scan(dir.path(), cache);
        assert!(idx2.set(930062).is_none());
        assert_eq!(cache2.len(), 2);
    }

    /// Scans the real lazer install (read-only). `cargo test live_scan -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_scan() {
        let data = data_dir().expect("lazer data dir");
        let t = std::time::Instant::now();
        let (idx, cache) = scan(&data, ScanCache::new());
        let cold = t.elapsed();
        let t = std::time::Instant::now();
        let _ = scan(&data, cache.clone());
        println!(
            "{}: {} files, {} sets, {} diffs; cold {:?}, cached {:?}",
            data.display(),
            cache.len(),
            idx.by_set.len(),
            idx.all.len(),
            cold,
            t.elapsed()
        );
        assert!(!idx.all.is_empty());
    }
}
