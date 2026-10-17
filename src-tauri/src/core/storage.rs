//! On-disk state: settings, import history and the collection cache. All JSON, written atomically.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::download::Mirror;
use super::osucollector::CollectionData;

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(value)?)?;
    std::fs::rename(tmp, path)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Manually chosen `osu!.exe`; `None` = autodetect
    pub lazer_path: Option<String>,
    pub mirrors: Vec<Mirror>,
    pub concurrency: u8,
    pub keep_osz: bool,
    /// Install whole sets instead of trimming
    pub no_trim: bool,
    /// When the mirror's version has none of the wanted hashes, install the whole set
    pub full_on_mismatch: bool,
    pub theme: String,
    /// Where trimmed `.osz` go in manual mode (lazer not found)
    pub manual_dir: Option<String>,
    pub replace_warning_seen: bool,
    /// Read lazer's file store (read-only) to skip difficulties that are already installed
    pub check_installed: bool,
    /// Folder handed to lazer's import wizard; `None` = Documents\Collector to Lazer
    pub export_dir: Option<String>,
    /// "ru" or "en"; `None` until the UI picks one from the system language
    pub language: Option<String>,
    /// Bumped when defaults change in a way old settings files must pick up; absent in v1 files
    #[serde(default)]
    pub version: u32,
}

const SETTINGS_VERSION: u32 = 2;

impl Default for Settings {
    fn default() -> Self {
        Self {
            lazer_path: None,
            mirrors: Mirror::ALL.to_vec(),
            concurrency: 10,
            keep_osz: false,
            no_trim: false,
            full_on_mismatch: true,
            theme: "dark".into(),
            manual_dir: None,
            replace_warning_seen: false,
            check_installed: true,
            export_dir: None,
            language: None,
            version: SETTINGS_VERSION,
        }
    }
}

impl Settings {
    pub fn sanitized(mut self) -> Self {
        if self.version < 2 {
            // v2: all mirrors are used in parallel; fastest first, sayobot added, more downloads at once
            self.mirrors = Mirror::ALL.to_vec();
            self.concurrency = 10;
        }
        self.version = SETTINGS_VERSION;
        self.concurrency = self.concurrency.clamp(1, 16);
        let mut seen = Vec::new();
        self.mirrors.retain(|m| {
            let fresh = !seen.contains(m);
            seen.push(*m);
            fresh
        });
        self.lazer_path = self.lazer_path.filter(|p| !p.trim().is_empty());
        self.export_dir = self.export_dir.filter(|p| !p.trim().is_empty());
        self.language = self.language.filter(|l| l == "ru" || l == "en");
        self
    }
}

/// Which difficulties of each set this app has already installed into lazer.
/// Needed because lazer replaces a set with the same online id on re-import.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct History {
    pub sets: BTreeMap<u64, BTreeSet<String>>,
}

impl History {
    pub fn installed(&self, set_id: u64) -> Option<&BTreeSet<String>> {
        self.sets.get(&set_id)
    }

    /// Hashes to keep in a new import of the set: what was installed before plus what is wanted now.
    pub fn union_with(&self, set_id: u64, wanted: &BTreeSet<String>) -> BTreeSet<String> {
        let mut all = wanted.clone();
        if let Some(prev) = self.sets.get(&set_id) {
            all.extend(prev.iter().cloned());
        }
        all
    }

    /// True when every wanted hash is already installed by us.
    pub fn covers(&self, set_id: u64, wanted: &BTreeSet<String>) -> bool {
        self.sets
            .get(&set_id)
            .is_some_and(|prev| wanted.is_subset(prev))
    }

    pub fn record(&mut self, set_id: u64, hashes: impl IntoIterator<Item = String>) {
        self.sets.entry(set_id).or_default().extend(hashes);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentEntry {
    pub id: u64,
    pub name: String,
    pub uploader: String,
    pub cover: Option<String>,
    pub diffs: u32,
    pub sets: u32,
    pub fetched_at: u64,
}

pub struct Paths {
    pub data: PathBuf,
    pub temp: PathBuf,
    /// The user's Documents folder, if known
    pub documents: Option<PathBuf>,
}

impl Paths {
    pub fn settings(&self) -> PathBuf {
        self.data.join("settings.json")
    }
    pub fn history(&self) -> PathBuf {
        self.data.join("history.json")
    }
    pub fn cache_dir(&self) -> PathBuf {
        self.data.join("cache")
    }
    pub fn cached_collection(&self, id: u64) -> PathBuf {
        self.cache_dir().join(format!("{id}.json"))
    }
    pub fn lazer_scan_cache(&self) -> PathBuf {
        self.cache_dir().join("lazer-scan.json")
    }
    pub fn md5_cache(&self) -> PathBuf {
        self.cache_dir().join("md5.json")
    }
    pub fn recent(&self) -> PathBuf {
        self.cache_dir().join("recent.json")
    }
    /// Easy to reach from lazer's built-in folder browser (unlike hidden AppData).
    pub fn default_export_dir(&self) -> PathBuf {
        match &self.documents {
            Some(d) => d.join("Collector to Lazer"),
            None => self.data.join("lazer-import"),
        }
    }
    pub fn export_dir(&self, settings: &Settings) -> PathBuf {
        settings
            .export_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.default_export_dir())
    }
    pub fn exported(&self) -> PathBuf {
        self.data.join("exported-collections.json")
    }
    pub fn kept_osz_dir(&self) -> PathBuf {
        self.data.join("osz")
    }
    pub fn downloads(&self) -> PathBuf {
        self.temp.join("downloads")
    }
}

pub fn save_collection(paths: &Paths, data: &CollectionData) -> std::io::Result<()> {
    write_json(&paths.cached_collection(data.info.id), data)?;
    let mut recent: Vec<RecentEntry> = read_json(&paths.recent()).unwrap_or_default();
    recent.retain(|r| r.id != data.info.id);
    let first_cover = data
        .beatmaps
        .first()
        .and_then(|b| data.sets.iter().find(|s| s.id == b.set_id))
        .and_then(|s| s.cover.clone());
    let set_count = data
        .beatmaps
        .iter()
        .map(|b| b.set_id)
        .collect::<std::collections::HashSet<_>>()
        .len();
    recent.insert(
        0,
        RecentEntry {
            id: data.info.id,
            name: data.info.name.clone(),
            uploader: data.info.uploader.clone(),
            cover: first_cover.or_else(|| data.sets.first().and_then(|s| s.cover.clone())),
            diffs: data.beatmaps.len() as u32,
            sets: set_count as u32,
            fetched_at: data.fetched_at,
        },
    );
    recent.truncate(24);
    write_json(&paths.recent(), &recent)
}

pub fn load_cached_collection(paths: &Paths, id: u64) -> Option<CollectionData> {
    read_json(&paths.cached_collection(id))
}

pub fn recent(paths: &Paths) -> Vec<RecentEntry> {
    let list: Vec<RecentEntry> = read_json(&paths.recent()).unwrap_or_default();
    list.into_iter()
        .filter(|r| paths.cached_collection(r.id).exists())
        .collect()
}

/// Every collection prepared for lazer so far, by name. Kept so one wizard pass imports all of them:
/// lazer merges by name and never duplicates, so re-importing old ones is harmless.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Exported {
    pub collections: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportedEntry {
    pub name: String,
    pub count: u32,
}

impl Exported {
    pub fn entries(&self) -> Vec<ExportedEntry> {
        self.collections
            .iter()
            .map(|(name, h)| ExportedEntry {
                name: name.clone(),
                count: h.len() as u32,
            })
            .collect()
    }
}

/// Writes `<dir>/collection.db` with all `collections` and an empty `Songs/`, so lazer's
/// "import from previous install" accepts the folder.
pub fn write_export(dir: &Path, exported: &Exported) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir.join("Songs"))?;
    let cols: Vec<super::collection_db::Collection> = exported
        .collections
        .iter()
        .map(|(name, h)| super::collection_db::Collection {
            name: name.clone(),
            hashes: h.iter().cloned().collect(),
        })
        .collect();
    let path = dir.join("collection.db");
    let tmp = dir.join("collection.db.tmp");
    std::fs::write(&tmp, super::collection_db::to_bytes(&cols))?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Adds `hashes` to collection `name` and rewrites the export folder with every collection.
pub fn merge_export(
    paths: &Paths,
    dir: &Path,
    name: &str,
    hashes: impl IntoIterator<Item = String>,
) -> std::io::Result<Exported> {
    let mut exported: Exported = read_json(&paths.exported()).unwrap_or_default();
    exported
        .collections
        .entry(name.to_string())
        .or_default()
        .extend(hashes);
    write_json(&paths.exported(), &exported)?;
    write_export(dir, &exported)?;
    Ok(exported)
}

/// Forgets all prepared collections and removes the export's `collection.db`.
pub fn clear_export(paths: &Paths, dir: &Path) -> std::io::Result<()> {
    write_json(&paths.exported(), &Exported::default())?;
    match std::fs::remove_file(dir.join("collection.db")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn history_union_keeps_previous_difficulties() {
        let mut h = History::default();
        h.record(10, set(&["seven"]));
        // later the user installs 6* from the same set: the new .osz must still contain the 7*
        assert_eq!(h.union_with(10, &set(&["six"])), set(&["seven", "six"]));
        assert_eq!(h.union_with(11, &set(&["x"])), set(&["x"]));
        assert!(h.covers(10, &set(&["seven"])));
        assert!(!h.covers(10, &set(&["seven", "six"])));
        assert!(!h.covers(99, &set(&["a"])));
    }

    #[test]
    fn json_round_trip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/history.json");
        let mut h = History::default();
        h.record(5, set(&["a", "b"]));
        write_json(&p, &h).unwrap();
        assert_eq!(read_json::<History>(&p).unwrap(), h);

        // missing fields fall back to defaults; bad values are clamped
        let s: Settings = serde_json::from_str(
            r#"{"version": 2, "concurrency": 50, "mirrors": ["osudirect", "osudirect", "catboy"]}"#,
        )
        .unwrap();
        let s = s.sanitized();
        assert_eq!(s.concurrency, 16);
        assert_eq!(s.mirrors, vec![Mirror::OsuDirect, Mirror::Catboy]);
        assert!(s.full_on_mismatch);

        // settings saved by v1 get the new mirror order and parallelism
        let old: Settings = serde_json::from_str(
            r#"{"concurrency": 3, "mirrors": ["catboy", "osudirect", "nerinyan"]}"#,
        )
        .unwrap();
        let old = old.sanitized();
        assert_eq!(old.mirrors, Mirror::ALL.to_vec());
        assert_eq!(old.concurrency, 10);
    }

    #[test]
    fn export_accumulates_collections_across_runs() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            data: dir.path().join("data"),
            temp: dir.path().join("tmp"),
            documents: None,
        };
        let out = dir.path().join("export");
        merge_export(&paths, &out, "My ★ collection", ["a".repeat(32)]).unwrap();
        merge_export(&paths, &out, "Second", ["b".repeat(32)]).unwrap();
        let ex = merge_export(
            &paths,
            &out,
            "My ★ collection",
            ["a".repeat(32), "c".repeat(32)],
        )
        .unwrap();
        assert_eq!(
            ex.entries(),
            vec![
                ExportedEntry {
                    name: "My ★ collection".into(),
                    count: 2
                },
                ExportedEntry {
                    name: "Second".into(),
                    count: 1
                },
            ]
        );
        assert!(out.join("Songs").is_dir());
        let cols = super::super::collection_db::from_bytes(
            &std::fs::read(out.join("collection.db")).unwrap(),
        )
        .unwrap();
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0].name, "My ★ collection");
        assert_eq!(cols[0].hashes.len(), 2);

        clear_export(&paths, &out).unwrap();
        assert!(!out.join("collection.db").exists());
        assert!(
            read_json::<Exported>(&paths.exported())
                .unwrap()
                .collections
                .is_empty()
        );
    }
}
