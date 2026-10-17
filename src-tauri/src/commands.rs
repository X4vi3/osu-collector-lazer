//! Tauri commands: the only surface the UI talks to.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, State};

use crate::core::filter::{self, Bucket};
use crate::core::job::{Job, JobOptions, Mode, Progress, SetSpec};
use crate::core::lazer_scan::{self, Installed, ScanSummary};
use crate::core::osucollector::{self, CollectionData, CollectionInfo, LoadProgress};
use crate::core::storage::{self, History, Paths, RecentEntry, Settings};
use crate::core::{collection_db, lazer, md5lookup};

/// Collections loaded from a collection.db get ids above any real osucollector id.
const DB_ID_BASE: u64 = 9_000_000_000_000;

pub struct AppState {
    pub paths: Arc<Paths>,
    pub client: reqwest::Client,
    pub settings: Mutex<Settings>,
    pub history: Arc<Mutex<History>>,
    pub collections: Mutex<HashMap<u64, Arc<CollectionData>>>,
    pub job: Mutex<Option<Arc<Job>>>,
    pub next_job_id: Mutex<u64>,
    pub md5_cache: Mutex<Option<Arc<Mutex<md5lookup::Md5Cache>>>>,
    pub lookup_cancel: Mutex<Option<tokio_util::sync::CancellationToken>>,
    pub next_db_id: Mutex<u64>,
    pub installed: Mutex<Option<Arc<Installed>>>,
    /// serialises scans so two callers don't read the store at once
    pub scan_lock: tokio::sync::Mutex<()>,
}

impl AppState {
    pub fn new(paths: Paths) -> Self {
        let settings = storage::read_json::<Settings>(&paths.settings())
            .unwrap_or_default()
            .sanitized();
        crate::core::i18n::set_language(settings.language.as_deref().unwrap_or("ru"));
        let history = storage::read_json::<History>(&paths.history()).unwrap_or_default();
        // leftovers of an interrupted run; anything lazer took is already gone
        let _ = std::fs::remove_dir_all(paths.downloads());
        migrate_old_export(&paths, &settings);
        Self {
            paths: Arc::new(paths),
            client: crate::core::http_client(),
            settings: Mutex::new(settings),
            history: Arc::new(Mutex::new(history)),
            collections: Mutex::new(HashMap::new()),
            job: Mutex::new(None),
            next_job_id: Mutex::new(1),
            md5_cache: Mutex::new(None),
            lookup_cancel: Mutex::new(None),
            next_db_id: Mutex::new(DB_ID_BASE),
            installed: Mutex::new(None),
            scan_lock: tokio::sync::Mutex::new(()),
        }
    }
}

type CmdResult<T> = Result<T, String>;

/// v0.1 wrote a single collection to `<data>/lazer-import`; carry it into the cumulative export.
fn migrate_old_export(paths: &Paths, settings: &Settings) {
    let old_db = paths.data.join("lazer-import").join("collection.db");
    if paths.exported().exists() || !old_db.exists() {
        return;
    }
    let Ok(cols) = std::fs::read(&old_db)
        .map_err(|_| ())
        .and_then(|b| collection_db::from_bytes(&b).map_err(|_| ()))
    else {
        return;
    };
    let mut exported = storage::Exported::default();
    for c in cols {
        exported
            .collections
            .entry(c.name)
            .or_default()
            .extend(c.hashes);
    }
    if storage::write_json(&paths.exported(), &exported).is_ok() {
        let _ = storage::write_export(&paths.export_dir(settings), &exported);
        let _ = std::fs::remove_dir_all(paths.data.join("lazer-import"));
    }
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: Settings) -> CmdResult<Settings> {
    let s = settings.sanitized();
    storage::write_json(&state.paths.settings(), &s).map_err(|e| {
        crate::trf!(
            "Не удалось сохранить настройки: {e}",
            "Could not save settings: {e}"
        )
    })?;
    crate::core::i18n::set_language(s.language.as_deref().unwrap_or("en"));
    let old = std::mem::replace(&mut *state.settings.lock().unwrap(), s.clone());
    let (old_dir, new_dir) = (state.paths.export_dir(&old), state.paths.export_dir(&s));
    if old_dir != new_dir {
        // move the prepared collections to the new folder
        let exported: storage::Exported =
            storage::read_json(&state.paths.exported()).unwrap_or_default();
        if !exported.collections.is_empty() {
            storage::write_export(&new_dir, &exported).map_err(|e| {
                crate::trf!("Папка недоступна: {e}", "Folder is not accessible: {e}")
            })?;
            let _ = std::fs::remove_file(old_dir.join("collection.db"));
        }
    }
    Ok(s)
}

#[tauri::command]
pub fn lazer_status(state: State<'_, AppState>) -> lazer::LazerStatus {
    let configured = state.settings.lock().unwrap().lazer_path.clone();
    lazer::status(configured.as_deref())
}

#[tauri::command]
pub fn detected_lazer_paths() -> Vec<String> {
    lazer::candidate_paths()
        .into_iter()
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string())
        .collect()
}

#[tauri::command]
pub async fn launch_lazer(state: State<'_, AppState>) -> CmdResult<()> {
    let configured = state.settings.lock().unwrap().lazer_path.clone();
    let exe = lazer::resolve_exe(configured.as_deref()).ok_or(crate::core::i18n::tr(
        "osu!lazer не найден. Укажите путь к osu!.exe в настройках.",
        "osu!lazer was not found. Set the path to osu!.exe in settings.",
    ))?;
    lazer::launch_and_wait(&exe, std::time::Duration::from_secs(90)).await
}

#[tauri::command]
pub fn recent_collections(state: State<'_, AppState>) -> Vec<RecentEntry> {
    storage::recent(&state.paths)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionView {
    pub info: CollectionInfo,
    pub fetched_at: u64,
    pub from_cache: bool,
    pub buckets: Vec<Bucket>,
    pub std_diffs: u32,
    pub std_sets: u32,
    /// For each set: bit N set if it has a difficulty in bucket N. Lets the UI count distinct sets for any selection.
    pub set_masks: Vec<u16>,
    pub cover: Option<String>,
    /// Loaded from a collection.db rather than osucollector.com
    pub from_db: bool,
}

fn view(data: &CollectionData, from_cache: bool) -> CollectionView {
    let mut masks: HashMap<u64, u16> = HashMap::new();
    for b in &data.beatmaps {
        *masks.entry(b.set_id).or_default() |= 1 << filter::bucket_of(b.sr);
    }
    let cover = data
        .beatmaps
        .first()
        .and_then(|b| data.sets.iter().find(|s| s.id == b.set_id))
        .and_then(|s| s.cover.clone());
    CollectionView {
        info: data.info.clone(),
        fetched_at: data.fetched_at,
        from_cache,
        buckets: filter::histogram(&data.beatmaps),
        std_diffs: data.beatmaps.len() as u32,
        std_sets: masks.len() as u32,
        set_masks: masks.into_values().collect(),
        cover,
        from_db: data.info.id >= DB_ID_BASE,
    }
}

#[tauri::command]
pub async fn load_collection(
    state: State<'_, AppState>,
    input: String,
    refresh: bool,
    on_progress: Channel<LoadProgress>,
) -> CmdResult<CollectionView> {
    let id = osucollector::parse_collection_id(&input).ok_or(crate::core::i18n::tr(
        "Не похоже на ссылку osucollector.com/collections/… или ID коллекции",
        "That does not look like an osucollector.com/collections/… link or a collection ID",
    ))?;

    if !refresh {
        if let Some(data) = state.collections.lock().unwrap().get(&id).cloned() {
            return Ok(view(&data, true));
        }
        let paths = state.paths.clone();
        let cached =
            tokio::task::spawn_blocking(move || storage::load_cached_collection(&paths, id))
                .await
                .ok()
                .flatten();
        if let Some(data) = cached {
            let v = view(&data, true);
            state.collections.lock().unwrap().insert(id, Arc::new(data));
            return Ok(v);
        }
    }

    let data = osucollector::fetch_collection(&state.client, id, |p| {
        let _ = on_progress.send(p);
    })
    .await
    .map_err(|e| e.to_string())?;
    let paths = state.paths.clone();
    let data = Arc::new(data);
    let d2 = data.clone();
    let _ = tokio::task::spawn_blocking(move || storage::save_collection(&paths, &d2)).await;
    let v = view(&data, false);
    state.collections.lock().unwrap().insert(id, data);
    Ok(v)
}

fn loaded(state: &AppState, id: u64) -> CmdResult<Arc<CollectionData>> {
    if let Some(d) = state.collections.lock().unwrap().get(&id).cloned() {
        return Ok(d);
    }
    let data = storage::load_cached_collection(&state.paths, id).ok_or(crate::core::i18n::tr(
        "Коллекция не загружена",
        "Collection is not loaded",
    ))?;
    let data = Arc::new(data);
    state.collections.lock().unwrap().insert(id, data.clone());
    Ok(data)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRow {
    pub set_id: u64,
    pub artist: String,
    pub title: String,
    pub version: String,
    pub sr: f64,
}

#[tauri::command]
pub fn selection_preview(
    state: State<'_, AppState>,
    collection_id: u64,
    buckets: Vec<u8>,
) -> CmdResult<Vec<PreviewRow>> {
    let data = loaded(&state, collection_id)?;
    let wanted: HashSet<u8> = buckets.into_iter().collect();
    let sets: HashMap<u64, &osucollector::BeatmapSet> =
        data.sets.iter().map(|s| (s.id, s)).collect();
    let mut rows: Vec<PreviewRow> = data
        .beatmaps
        .iter()
        .filter(|b| wanted.contains(&filter::bucket_of(b.sr)))
        .map(|b| {
            let s = sets.get(&b.set_id);
            PreviewRow {
                set_id: b.set_id,
                artist: s.map(|s| s.artist.clone()).unwrap_or_default(),
                title: s
                    .map(|s| s.title.clone())
                    .unwrap_or_else(|| format!("#{}", b.set_id)),
                version: b.version.clone(),
                sr: b.sr,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        a.artist
            .to_lowercase()
            .cmp(&b.artist.to_lowercase())
            .then(a.title.cmp(&b.title))
            .then(a.sr.total_cmp(&b.sr))
    });
    Ok(rows)
}

fn emit_progress(app: AppHandle) -> crate::core::job::Emit {
    Arc::new(move |p: Progress| {
        let _ = app.emit("job-progress", p);
    })
}

#[tauri::command]
pub async fn start_job(
    app: AppHandle,
    state: State<'_, AppState>,
    collection_id: u64,
    buckets: Vec<u8>,
    manual_dir: Option<String>,
) -> CmdResult<u64> {
    if let Some(j) = state.job.lock().unwrap().as_ref() {
        if !j.is_finished() {
            return Err(crate::core::i18n::tr(
                "Уже идёт загрузка. Дождитесь окончания или отмените её.",
                "A download is already running. Wait for it to finish or cancel it.",
            )
            .into());
        }
    }
    let data = loaded(&state, collection_id)?;
    let settings = state.settings.lock().unwrap().clone();

    let mode = match manual_dir {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).map_err(|e| {
                crate::trf!("Папка недоступна: {e}", "Folder is not accessible: {e}")
            })?;
            Mode::Manual { dir }
        }
        None => {
            let exe = lazer::resolve_exe(settings.lazer_path.as_deref())
                .ok_or(crate::core::i18n::tr("osu!lazer не найден. Укажите путь к osu!.exe в настройках или используйте ручной режим.", "osu!lazer was not found. Set the path to osu!.exe in settings or use manual mode."))?;
            Mode::Import { exe }
        }
    };
    if settings.mirrors.is_empty() {
        return Err(crate::core::i18n::tr(
            "В настройках не выбрано ни одного зеркала",
            "No mirrors are enabled in settings",
        )
        .into());
    }

    let selection = filter::select(&data.beatmaps, &buckets);
    if selection.by_set.is_empty() {
        return Err(crate::core::i18n::tr("Ничего не выбрано", "Nothing is selected").into());
    }
    let sets: HashMap<u64, &osucollector::BeatmapSet> =
        data.sets.iter().map(|s| (s.id, s)).collect();
    let specs: Vec<SetSpec> = selection
        .by_set
        .into_iter()
        .map(|(set_id, wanted)| {
            let meta = sets.get(&set_id);
            SetSpec {
                set_id,
                artist: meta.map(|s| s.artist.clone()).unwrap_or_default(),
                title: meta
                    .map(|s| s.title.clone())
                    .unwrap_or_else(|| format!("#{set_id}")),
                wanted: wanted.into_iter().collect::<BTreeSet<_>>(),
            }
        })
        .collect();

    let id = {
        let mut n = state.next_job_id.lock().unwrap();
        *n += 1;
        *n
    };
    let installed = if matches!(mode, Mode::Import { .. }) && settings.check_installed {
        // fresh scan: the user may have installed maps since the collection was opened
        rescan(&state).await.ok().flatten()
    } else {
        None
    };
    let opts = JobOptions {
        installed,
        export_dir: state.paths.export_dir(&settings),
        mirrors: settings.mirrors.clone(),
        concurrency: settings.concurrency as usize,
        keep_osz: settings.keep_osz,
        no_trim: settings.no_trim,
        full_on_mismatch: settings.full_on_mismatch,
    };
    let job = Job::new(
        id,
        data.info.name.clone(),
        specs,
        mode,
        opts,
        state.paths.clone(),
        state.client.clone(),
        state.history.clone(),
        emit_progress(app),
    );
    job.start();
    *state.job.lock().unwrap() = Some(job);
    Ok(id)
}

#[tauri::command]
pub fn job_snapshot(state: State<'_, AppState>) -> Option<Progress> {
    state.job.lock().unwrap().as_ref().map(|j| j.snapshot())
}

fn with_job(state: &AppState, f: impl FnOnce(&Arc<Job>)) -> CmdResult<()> {
    let job = state
        .job
        .lock()
        .unwrap()
        .clone()
        .ok_or(crate::core::i18n::tr(
            "Нет активной загрузки",
            "No active download",
        ))?;
    f(&job);
    Ok(())
}

#[tauri::command]
pub fn pause_job(state: State<'_, AppState>) -> CmdResult<()> {
    with_job(&state, |j| j.pause())
}

#[tauri::command]
pub fn resume_job(state: State<'_, AppState>) -> CmdResult<()> {
    with_job(&state, |j| j.resume())
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>) -> CmdResult<()> {
    with_job(&state, |j| j.cancel())
}

#[tauri::command]
pub async fn retry_failed(state: State<'_, AppState>) -> CmdResult<bool> {
    let job = state
        .job
        .lock()
        .unwrap()
        .clone()
        .ok_or(crate::core::i18n::tr("Нет загрузки", "No download"))?;
    Ok(job.retry_failed())
}

#[tauri::command]
pub fn clear_cache(state: State<'_, AppState>) -> CmdResult<()> {
    state.collections.lock().unwrap().clear();
    *state.md5_cache.lock().unwrap() = None;
    match std::fs::remove_dir_all(state.paths.cache_dir()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(crate::trf!(
            "Не удалось очистить кэш: {e}",
            "Could not clear the cache: {e}"
        )),
    }
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) -> CmdResult<()> {
    *state.history.lock().unwrap() = History::default();
    storage::write_json(&state.paths.history(), &History::default()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn history_size(state: State<'_, AppState>) -> usize {
    state.history.lock().unwrap().sets.len()
}

#[tauri::command]
pub fn open_folder(app: AppHandle, path: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbCollection {
    pub name: String,
    pub count: usize,
}

/// Phase 6: list collections inside a dropped `collection.db`.
#[tauri::command]
pub fn read_collection_db(path: String) -> CmdResult<Vec<DbCollection>> {
    let bytes = std::fs::read(&path).map_err(|e| {
        crate::trf!(
            "Не удалось прочитать файл: {e}",
            "Could not read the file: {e}"
        )
    })?;
    let cols = collection_db::from_bytes(&bytes).map_err(|e| e.to_string())?;
    Ok(cols
        .into_iter()
        .map(|c| DbCollection {
            name: c.name,
            count: c.hashes.len(),
        })
        .collect())
}

/// Phase 6: resolve one collection of a `collection.db` through mirror MD5 lookups.
#[tauri::command]
pub async fn load_db_collection(
    state: State<'_, AppState>,
    path: String,
    index: usize,
    on_progress: Channel<md5lookup::LookupProgress>,
) -> CmdResult<CollectionView> {
    let bytes = std::fs::read(&path).map_err(|e| {
        crate::trf!(
            "Не удалось прочитать файл: {e}",
            "Could not read the file: {e}"
        )
    })?;
    let cols = collection_db::from_bytes(&bytes).map_err(|e| e.to_string())?;
    let col = cols.into_iter().nth(index).ok_or(crate::core::i18n::tr(
        "Нет такой коллекции в файле",
        "No such collection in the file",
    ))?;
    let mut hashes: Vec<String> = col.hashes.iter().map(|h| h.to_ascii_lowercase()).collect();
    hashes.sort();
    hashes.dedup();

    let cache = {
        let mut slot = state.md5_cache.lock().unwrap();
        slot.get_or_insert_with(|| {
            Arc::new(Mutex::new(
                storage::read_json(&state.paths.md5_cache()).unwrap_or_default(),
            ))
        })
        .clone()
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    *state.lookup_cancel.lock().unwrap() = Some(cancel.clone());

    md5lookup::lookup_all(
        &state.client,
        &hashes,
        cache.clone(),
        cancel.clone(),
        move |p| {
            let _ = on_progress.send(p);
        },
    )
    .await;

    let snapshot = cache.lock().unwrap().clone();
    let paths = state.paths.clone();
    let _ = tokio::task::spawn_blocking(move || storage::write_json(&paths.md5_cache(), &snapshot))
        .await;
    if cancel.is_cancelled() {
        return Err(crate::core::i18n::tr("Поиск остановлен. Уже найденные карты сохранены, при следующей попытке продолжится с того же места.", "Lookup stopped. Maps found so far are saved; the next attempt continues where it left off.").into());
    }

    let c = cache.lock().unwrap();
    let mut beatmaps = Vec::new();
    let mut sets: HashMap<u64, osucollector::BeatmapSet> = HashMap::new();
    for h in &hashes {
        if let Some(Some(f)) = c.get(h) {
            if !f.std {
                continue;
            }
            beatmaps.push(f.beatmap.clone());
            let set = f.set.clone().unwrap_or_else(|| osucollector::BeatmapSet {
                id: f.beatmap.set_id,
                artist: String::new(),
                title: format!("#{}", f.beatmap.set_id),
                creator: String::new(),
                cover: Some(format!(
                    "https://assets.ppy.sh/beatmaps/{}/covers/card.jpg",
                    f.beatmap.set_id
                )),
            });
            sets.entry(set.id).or_insert(set);
        }
    }
    drop(c);
    let id = {
        let mut n = state.next_db_id.lock().unwrap();
        *n += 1;
        *n
    };
    let data = CollectionData {
        info: CollectionInfo {
            id,
            name: if col.name.is_empty() {
                crate::core::i18n::tr(
                    "Коллекция из collection.db",
                    "Collection from collection.db",
                )
                .into()
            } else {
                col.name
            },
            uploader: String::new(),
            beatmap_count: hashes.len() as u64,
        },
        beatmaps,
        sets: sets.into_values().collect(),
        fetched_at: osucollector::now_secs(),
    };
    let v = view(&data, false);
    state.collections.lock().unwrap().insert(id, Arc::new(data));
    Ok(v)
}

#[tauri::command]
pub fn cancel_db_lookup(state: State<'_, AppState>) {
    if let Some(c) = state.lookup_cancel.lock().unwrap().take() {
        c.cancel();
    }
}

/// Re-reads lazer's file store (read-only, cached by file hash). `None` if lazer data isn't found.
async fn rescan(state: &AppState) -> CmdResult<Option<Arc<Installed>>> {
    let _guard = state.scan_lock.lock().await;
    let Some(data) = lazer_scan::data_dir() else {
        *state.installed.lock().unwrap() = None;
        return Ok(None);
    };
    let paths = state.paths.clone();
    let idx = tokio::task::spawn_blocking(move || {
        let cache = storage::read_json(&paths.lazer_scan_cache()).unwrap_or_default();
        let (idx, cache) = lazer_scan::scan(&data, cache);
        let _ = storage::write_json(&paths.lazer_scan_cache(), &cache);
        idx
    })
    .await
    .map_err(|e| e.to_string())?;
    let idx = Arc::new(idx);
    *state.installed.lock().unwrap() = Some(idx.clone());
    Ok(Some(idx))
}

#[tauri::command]
pub async fn scan_lazer(state: State<'_, AppState>) -> CmdResult<ScanSummary> {
    let idx = rescan(&state).await?;
    Ok(ScanSummary {
        found: idx.is_some(),
        path: lazer_scan::data_dir().map(|p| p.display().to_string()),
        sets: idx.as_ref().map_or(0, |i| i.by_set.len() as u32),
        diffs: idx.as_ref().map_or(0, |i| i.all.len() as u32),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledCount {
    /// selected difficulties already in lazer (or installed earlier by this app)
    pub diffs: u32,
    /// selected sets whose every selected difficulty is already there
    pub sets: u32,
}

#[tauri::command]
pub fn installed_count(
    state: State<'_, AppState>,
    collection_id: u64,
    buckets: Vec<u8>,
) -> CmdResult<InstalledCount> {
    let data = loaded(&state, collection_id)?;
    let installed = state.installed.lock().unwrap().clone();
    let history = state.history.lock().unwrap();
    let sel = filter::select(&data.beatmaps, &buckets);
    let mut out = InstalledCount { diffs: 0, sets: 0 };
    for (set_id, wanted) in &sel.by_set {
        let have = |h: &String| {
            installed.as_ref().is_some_and(|i| i.all.contains(h))
                || history.installed(*set_id).is_some_and(|s| s.contains(h))
        };
        let n = wanted.iter().filter(|h| have(h)).count();
        out.diffs += n as u32;
        if n == wanted.len() {
            out.sets += 1;
        }
    }
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportInfo {
    pub dir: String,
    pub default_dir: String,
    pub collections: Vec<storage::ExportedEntry>,
}

/// The folder for lazer's import wizard and the collections waiting in it.
#[tauri::command]
pub fn export_info(state: State<'_, AppState>) -> ExportInfo {
    let settings = state.settings.lock().unwrap().clone();
    let exported: storage::Exported =
        storage::read_json(&state.paths.exported()).unwrap_or_default();
    ExportInfo {
        dir: state.paths.export_dir(&settings).display().to_string(),
        default_dir: state.paths.default_export_dir().display().to_string(),
        collections: exported.entries(),
    }
}

#[tauri::command]
pub fn clear_export(state: State<'_, AppState>) -> CmdResult<()> {
    let settings = state.settings.lock().unwrap().clone();
    storage::clear_export(&state.paths, &state.paths.export_dir(&settings))
        .map_err(|e| e.to_string())
}

/// True when running from the NSIS install (it puts `uninstall.exe` next to the app).
/// The portable exe can't replace itself, so it gets a download link instead of auto-update.
#[tauri::command]
pub fn is_installed() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("uninstall.exe").is_file()))
        .unwrap_or(false)
}
