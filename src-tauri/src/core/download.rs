//! Beatmapset downloads from public mirrors.
//!
//! Mirrors throttle per connection (catboy ≈ 0.5 MB/s each) and per IP (osu.direct: 120 req/min,
//! burst 25/5 s), so all enabled mirrors are used at once, each with its own parallel cap.
//! A rate-limited mirror cools down while other mirrors keep downloading.
//! A set crawling from a slow mirror is raced by another free mirror, so one bad server
//! cannot hold the tail of a collection hostage.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio_util::sync::CancellationToken;

use super::trim;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mirror {
    Catboy,
    OsuDirect,
    Sayobot,
    Nerinyan,
}

impl Mirror {
    /// Default preference order: fastest first (measured from RU, 2026-10).
    pub const ALL: [Mirror; 4] = [
        Mirror::OsuDirect,
        Mirror::Catboy,
        Mirror::Sayobot,
        Mirror::Nerinyan,
    ];

    /// Download URL without video (each mirror has its own flag, verified 2026-10).
    pub fn url(self, set_id: u64) -> String {
        match self {
            Mirror::Catboy => format!("https://catboy.best/d/{set_id}n"),
            Mirror::OsuDirect => format!("https://osu.direct/api/d/{set_id}?noVideo=1"),
            Mirror::Sayobot => format!("https://dl.sayobot.cn/beatmaps/download/novideo/{set_id}"),
            Mirror::Nerinyan => format!("https://api.nerinyan.moe/d/{set_id}?noVideo=true"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mirror::Catboy => "catboy.best",
            Mirror::OsuDirect => "osu.direct",
            Mirror::Sayobot => "sayobot.cn",
            Mirror::Nerinyan => "nerinyan.moe",
        }
    }

    /// Simultaneous downloads this mirror tolerates from one IP.
    pub fn max_parallel(self) -> usize {
        match self {
            Mirror::OsuDirect => 4,
            Mirror::Catboy => 6,
            Mirror::Sayobot => 4,
            Mirror::Nerinyan => 2,
        }
    }
}

/// Pause/cancel signals shared by all workers of a job.
#[derive(Clone)]
pub struct Control {
    pub cancel: CancellationToken,
    pub paused: watch::Receiver<bool>,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum DownloadError {
    #[error("{}", crate::core::i18n::tr("отменено", "cancelled"))]
    Cancelled,
    #[error(
        "{}",
        crate::core::i18n::tr(
            "сет удалён или недоступен на всех зеркалах",
            "the set was deleted or is unavailable on every mirror"
        )
    )]
    NotFound,
    #[error(
        "{}",
        crate::core::i18n::tr("нет связи с зеркалами", "mirrors are unreachable")
    )]
    Network,
    #[error("{mirror}: {reason}")]
    Failed {
        mirror: &'static str,
        reason: String,
    },
}

impl Control {
    /// Waits while the job is paused. Returns `Cancelled` if cancelled meanwhile.
    pub async fn checkpoint(&self) -> Result<(), DownloadError> {
        let mut rx = self.paused.clone();
        loop {
            if self.cancel.is_cancelled() {
                return Err(DownloadError::Cancelled);
            }
            if !*rx.borrow_and_update() {
                return Ok(());
            }
            tokio::select! {
                _ = self.cancel.cancelled() => return Err(DownloadError::Cancelled),
                r = rx.changed() => if r.is_err() { return Ok(()) },
            }
        }
    }

    async fn sleep(&self, d: Duration) -> Result<(), DownloadError> {
        tokio::select! {
            _ = self.cancel.cancelled() => Err(DownloadError::Cancelled),
            _ = tokio::time::sleep(d) => Ok(()),
        }
    }
}

struct Slot {
    mirror: Mirror,
    sem: Arc<Semaphore>,
    cooldown_until: Mutex<Instant>,
}

/// Shared by all workers of a job: per-mirror parallel caps and rate-limit cooldowns.
pub struct MirrorPool {
    slots: Vec<Slot>,
}

impl MirrorPool {
    pub fn new(order: &[Mirror]) -> Self {
        let slots = order
            .iter()
            .map(|&m| Slot {
                mirror: m,
                sem: Arc::new(Semaphore::new(m.max_parallel())),
                cooldown_until: Mutex::new(Instant::now()),
            })
            .collect();
        Self { slots }
    }

    /// Total simultaneous downloads the enabled mirrors allow.
    pub fn capacity(&self) -> usize {
        self.slots.iter().map(|s| s.mirror.max_parallel()).sum()
    }

    pub fn mirrors(&self) -> impl Iterator<Item = Mirror> + '_ {
        self.slots.iter().map(|s| s.mirror)
    }

    fn cool_down(&self, m: Mirror, d: Duration) {
        if let Some(s) = self.slots.iter().find(|s| s.mirror == m) {
            let mut until = s.cooldown_until.lock().unwrap();
            *until = (*until).max(Instant::now() + d);
        }
    }

    /// First mirror in preference order that is not excluded, not cooling down and has a free slot now.
    fn try_acquire(&self, exclude: &HashSet<Mirror>) -> Option<(Mirror, OwnedSemaphorePermit)> {
        let now = Instant::now();
        self.slots
            .iter()
            .filter(|s| !exclude.contains(&s.mirror))
            .filter(|s| *s.cooldown_until.lock().unwrap() <= now)
            .find_map(|s| Some((s.mirror, s.sem.clone().try_acquire_owned().ok()?)))
    }

    /// Like `try_acquire`, but waits while all candidates are busy. `None` when every mirror is excluded.
    async fn acquire(
        &self,
        exclude: &HashSet<Mirror>,
        ctl: &Control,
    ) -> Result<Option<(Mirror, OwnedSemaphorePermit)>, DownloadError> {
        loop {
            if self.slots.iter().all(|s| exclude.contains(&s.mirror)) {
                return Ok(None);
            }
            if let Some(got) = self.try_acquire(exclude) {
                return Ok(Some(got));
            }
            ctl.sleep(Duration::from_millis(150)).await?;
        }
    }
}

/// Bytes received by one attempt, read by `download_set` while the attempt streams.
#[derive(Default)]
struct Live {
    done: AtomicU64,
    /// 0 = unknown
    total: AtomicU64,
}

impl Live {
    fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| (self.done.load(Ordering::Relaxed) as f64 / total as f64) as f32)
    }
}

/// One mirror currently downloading the set.
struct Running {
    live: Arc<Live>,
    cancel: CancellationToken,
    /// bytes already passed to `progress`
    seen: u64,
    /// start of the current speed window: (when, bytes done then)
    mark: (Instant, u64),
}

/// How often running attempts are sampled for progress and speed.
const SAMPLE: Duration = Duration::from_millis(250);
/// Speed is judged over windows this long.
const HEDGE_WINDOW: Duration = Duration::from_secs(10);
/// Slower than this, a second mirror is raced against the first (catboy alone gives ≈ 500 KB/s).
const HEDGE_SLOW_BPS: u64 = 150 * 1024;
/// ...unless the download is about to finish anyway.
const HEDGE_MIN_ETA: Duration = Duration::from_secs(20);

/// The current speed window has run long and the attempt is crawling with a lot left.
fn is_slow(r: &Running, now: Instant) -> bool {
    let (since, at) = r.mark;
    let age = now - since;
    if age < HEDGE_WINDOW {
        return false;
    }
    let done = r.live.done.load(Ordering::Relaxed);
    let bps = (done - at) as f64 / age.as_secs_f64();
    if bps >= HEDGE_SLOW_BPS as f64 {
        return false;
    }
    let total = r.live.total.load(Ordering::Relaxed);
    total == 0
        || bps <= 0.0
        || (total.saturating_sub(done)) as f64 / bps > HEDGE_MIN_ETA.as_secs_f64()
}

enum Attempt {
    Ok,
    /// 429: the mirror should cool down for this long
    RateLimited(Duration),
    /// transient (network, 5xx): try again
    Retry {
        reason: String,
        transport: bool,
    },
    /// Give up on this mirror for this set
    NextMirror {
        reason: String,
        not_found: bool,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum Outcome {
    NotFound,
    Transport,
    Other,
}

/// Header seconds: `Retry-After`, or osu.direct's `ratelimit-reset` when its window is spent.
fn rate_limit_wait(resp: &reqwest::Response) -> Duration {
    let h = resp.headers();
    let secs = |name: &str| h.get(name)?.to_str().ok()?.trim().parse::<u64>().ok();
    let s = secs("retry-after")
        .or_else(|| {
            (secs("ratelimit-burst-remaining") == Some(0))
                .then(|| secs("ratelimit-burst-reset"))
                .flatten()
        })
        .or_else(|| secs("ratelimit-reset"))
        .unwrap_or(10);
    Duration::from_secs(s.clamp(2, 120))
}

/// Passes bytes received since the last call to `progress`, with the fraction done by the attempt
/// furthest along; `on_mirror` hears whenever that leading attempt changes.
fn report(
    running: &mut HashMap<Mirror, Running>,
    shown: &mut (Option<Mirror>, Option<f32>),
    on_mirror: &mut impl FnMut(Mirror),
    progress: &mut impl FnMut(u64, Option<f32>),
) {
    let mut new = 0;
    for r in running.values_mut() {
        let done = r.live.done.load(Ordering::Relaxed);
        new += done - r.seen;
        r.seen = done;
    }
    let frac = |m: &Mirror| running[m].live.fraction().unwrap_or(0.0);
    let mut leader = shown.0.filter(|m| running.contains_key(m));
    for m in running.keys() {
        if leader.is_none_or(|l| frac(m) > frac(&l)) {
            leader = Some(*m);
        }
    }
    let Some(leader) = leader else { return };
    let f = running[&leader].live.fraction();
    if shown.0 != Some(leader) {
        on_mirror(leader);
    }
    if new > 0 || f != shown.1 {
        progress(new, f);
    }
    *shown = (Some(leader), f);
}

/// Cancels every running attempt and waits for them to clean up their partial files.
async fn stop_all<F: Future>(running: &HashMap<Mirror, Running>, tasks: &mut FuturesUnordered<F>) {
    for r in running.values() {
        r.cancel.cancel();
    }
    while tasks.next().await.is_some() {}
}

/// Downloads `set_id` into `dest` using whichever mirror is free, falling back across mirrors.
/// A download that crawls is raced by a second free mirror; whichever finishes first wins.
/// Calls `progress(new_bytes, fraction)` while streaming. Returns the mirror that succeeded.
pub async fn download_set(
    client: &reqwest::Client,
    set_id: u64,
    pool: &MirrorPool,
    dest: &Path,
    ctl: &Control,
    mut on_mirror: impl FnMut(Mirror),
    mut progress: impl FnMut(u64, Option<f32>),
) -> Result<Mirror, DownloadError> {
    let mut exhausted: HashSet<Mirror> = HashSet::new();
    let mut attempts: HashMap<Mirror, u32> = HashMap::new();
    let mut outcomes: HashMap<Mirror, Outcome> = HashMap::new();
    let mut last: Option<DownloadError> = None;
    let mut backoff: Option<Duration> = None;
    let mut running: HashMap<Mirror, Running> = HashMap::new();
    let mut tasks = FuturesUnordered::new();
    let mut shown: (Option<Mirror>, Option<f32>) = (None, None);
    let mut tick = tokio::time::interval(SAMPLE);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // every attempt writes its own .part file, so two mirrors can race on one set
    let launch =
        |mirror: Mirror, permit: OwnedSemaphorePermit, running: &mut HashMap<Mirror, Running>| {
            let live = Arc::new(Live::default());
            let cancel = ctl.cancel.child_token();
            running.insert(
                mirror,
                Running {
                    live: live.clone(),
                    cancel: cancel.clone(),
                    seen: 0,
                    mark: (Instant::now(), 0),
                },
            );
            let actl = Control {
                cancel,
                paused: ctl.paused.clone(),
            };
            let part = dest.with_extension(format!("{mirror:?}.part"));
            let url = mirror.url(set_id);
            async move {
                let r = try_once(client, url, &part, dest, &actl, &live).await;
                drop(permit);
                (mirror, r)
            }
        };

    loop {
        if tasks.is_empty() {
            if let Some(d) = backoff.take() {
                ctl.sleep(d).await?;
            }
            ctl.checkpoint().await?;
            let Some((mirror, permit)) = pool.acquire(&exhausted, ctl).await? else {
                break;
            };
            tasks.push(launch(mirror, permit, &mut running));
        }
        let (mirror, res) = tokio::select! {
            Some(finished) = tasks.next() => finished,
            _ = tick.tick() => {
                report(&mut running, &mut shown, &mut on_mirror, &mut progress);
                let now = Instant::now();
                let paused = *ctl.paused.borrow();
                let mut hedge_from = None;
                for (&m, r) in running.iter_mut() {
                    if paused || now - r.mark.0 >= HEDGE_WINDOW {
                        // a paused attempt is not slow: its window restarts
                        if !paused && is_slow(r, now) {
                            hedge_from = Some(m);
                        }
                        r.mark = (now, r.live.done.load(Ordering::Relaxed));
                    }
                }
                if let Some(m) = hedge_from.filter(|_| running.len() == 1) {
                    let mut exclude = exhausted.clone();
                    exclude.insert(m);
                    if let Some((m2, permit)) = pool.try_acquire(&exclude) {
                        tasks.push(launch(m2, permit, &mut running));
                    }
                }
                continue;
            }
        };
        report(&mut running, &mut shown, &mut on_mirror, &mut progress);
        running.remove(&mirror);
        let res = match res {
            Ok(a) => a,
            Err(e) => {
                stop_all(&running, &mut tasks).await;
                return Err(e);
            }
        };
        let n = attempts.entry(mirror).or_default();
        *n += 1;
        match res {
            Attempt::Ok => {
                stop_all(&running, &mut tasks).await;
                return Ok(mirror);
            }
            Attempt::RateLimited(wait) => {
                pool.cool_down(mirror, wait);
                last = Some(DownloadError::Failed {
                    mirror: mirror.label(),
                    reason: crate::core::i18n::tr("лимит запросов (429)", "rate limited (429)")
                        .into(),
                });
                outcomes.insert(mirror, Outcome::Other);
                if *n >= 6 {
                    exhausted.insert(mirror);
                }
            }
            Attempt::Retry { reason, transport } => {
                outcomes.insert(
                    mirror,
                    if transport {
                        Outcome::Transport
                    } else {
                        Outcome::Other
                    },
                );
                last = Some(DownloadError::Failed {
                    mirror: mirror.label(),
                    reason,
                });
                if *n >= 3 {
                    exhausted.insert(mirror);
                } else if tasks.is_empty() {
                    backoff = Some(Duration::from_secs(1 << *n));
                }
            }
            Attempt::NextMirror { reason, not_found } => {
                outcomes.insert(
                    mirror,
                    if not_found {
                        Outcome::NotFound
                    } else {
                        Outcome::Other
                    },
                );
                last = Some(DownloadError::Failed {
                    mirror: mirror.label(),
                    reason,
                });
                exhausted.insert(mirror);
            }
        }
    }

    let _ = tokio::fs::remove_file(dest).await;
    if outcomes.is_empty() {
        return Err(DownloadError::Failed {
            mirror: "-",
            reason: crate::core::i18n::tr("не выбрано ни одного зеркала", "no mirrors enabled")
                .into(),
        });
    }
    // some mirrors say "no such set", the rest only errored: it is gone, not a hiccup
    if outcomes.values().any(|o| *o == Outcome::NotFound)
        && !outcomes.values().any(|o| *o == Outcome::Transport)
    {
        return Err(DownloadError::NotFound);
    }
    if outcomes.values().all(|o| *o == Outcome::Transport) {
        return Err(DownloadError::Network);
    }
    Err(last.unwrap_or(DownloadError::NotFound))
}

async fn try_once(
    client: &reqwest::Client,
    url: String,
    part: &Path,
    dest: &Path,
    ctl: &Control,
    live: &Live,
) -> Result<Attempt, DownloadError> {
    let resp = tokio::select! {
        _ = ctl.cancel.cancelled() => return Err(DownloadError::Cancelled),
        r = client.get(&url).send() => r,
    };
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            return Ok(Attempt::Retry {
                reason: crate::trf!("сеть: {}", "network: {}", short_err(&e)),
                transport: true,
            });
        }
    };
    let status = resp.status();
    if status.as_u16() == 429 {
        return Ok(Attempt::RateLimited(rate_limit_wait(&resp)));
    }
    if status.is_server_error() {
        return Ok(Attempt::Retry {
            reason: crate::trf!("ошибка сервера {status}", "server error {status}"),
            transport: false,
        });
    }
    if status.as_u16() == 404 || status.as_u16() == 451 {
        return Ok(Attempt::NextMirror {
            reason: crate::trf!(
                "нет на зеркале ({})",
                "not on this mirror ({})",
                status.as_u16()
            ),
            not_found: true,
        });
    }
    if !status.is_success() {
        return Ok(Attempt::NextMirror {
            reason: crate::trf!("ответ {status}", "response {status}"),
            not_found: false,
        });
    }

    live.total
        .store(resp.content_length().unwrap_or(0), Ordering::Relaxed);
    let mut file = tokio::fs::File::create(part)
        .await
        .map_err(|e| DownloadError::Failed {
            mirror: "-",
            reason: crate::trf!("не удалось создать файл: {e}", "could not create file: {e}"),
        })?;
    let mut stream = resp.bytes_stream();
    loop {
        if let Err(e) = ctl.checkpoint().await {
            drop(file);
            let _ = tokio::fs::remove_file(part).await;
            return Err(e);
        }
        let chunk = tokio::select! {
            _ = ctl.cancel.cancelled() => None,
            c = stream.next() => Some(c),
        };
        let Some(chunk) = chunk else {
            drop(file);
            let _ = tokio::fs::remove_file(part).await;
            return Err(DownloadError::Cancelled);
        };
        match chunk {
            None => break,
            Some(Ok(bytes)) => {
                if let Err(e) = file.write_all(&bytes).await {
                    drop(file);
                    let _ = tokio::fs::remove_file(part).await;
                    return Err(DownloadError::Failed {
                        mirror: "-",
                        reason: crate::trf!("запись на диск: {e}", "disk write: {e}"),
                    });
                }
                live.done.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            }
            Some(Err(e)) => {
                drop(file);
                let _ = tokio::fs::remove_file(part).await;
                return Ok(Attempt::Retry {
                    reason: crate::trf!(
                        "сеть: обрыв загрузки ({})",
                        "network: download interrupted ({})",
                        short_err(&e)
                    ),
                    transport: true,
                });
            }
        }
    }
    let _ = file.flush().await;
    drop(file);

    let check = part.to_path_buf();
    let valid = tokio::task::spawn_blocking(move || trim::osu_hashes(&check)).await;
    match valid {
        // another mirror already won the race
        Ok(Ok(_)) if ctl.cancel.is_cancelled() => {
            let _ = tokio::fs::remove_file(part).await;
            Err(DownloadError::Cancelled)
        }
        Ok(Ok(_)) => {
            tokio::fs::rename(part, dest)
                .await
                .map_err(|e| DownloadError::Failed {
                    mirror: "-",
                    reason: crate::trf!(
                        "не удалось сохранить файл: {e}",
                        "could not save file: {e}"
                    ),
                })?;
            Ok(Attempt::Ok)
        }
        Ok(Err(e)) => {
            let _ = tokio::fs::remove_file(part).await;
            Ok(Attempt::NextMirror {
                reason: crate::trf!("плохой архив: {e}", "bad archive: {e}"),
                not_found: false,
            })
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(part).await;
            Ok(Attempt::NextMirror {
                reason: e.to_string(),
                not_found: false,
            })
        }
    }
}

fn short_err(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        crate::core::i18n::tr("таймаут", "timeout").into()
    } else if e.is_connect() {
        crate::core::i18n::tr("нет соединения", "no connection").into()
    } else {
        let s = e.to_string();
        s.chars().take(120).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl() -> (Control, watch::Sender<bool>) {
        let (tx, rx) = watch::channel(false);
        (
            Control {
                cancel: CancellationToken::new(),
                paused: rx,
            },
            tx,
        )
    }

    #[test]
    fn mirror_urls_request_no_video() {
        assert_eq!(Mirror::Catboy.url(5), "https://catboy.best/d/5n");
        assert!(Mirror::OsuDirect.url(5).ends_with("/api/d/5?noVideo=1"));
        assert!(Mirror::Sayobot.url(5).contains("/novideo/5"));
        assert!(Mirror::Nerinyan.url(5).contains("noVideo"));
    }

    #[tokio::test]
    async fn checkpoint_blocks_while_paused_and_honours_cancel() {
        let (c, tx) = ctl();
        c.checkpoint().await.unwrap();
        tx.send(true).unwrap();
        let c2 = c.clone();
        let h = tokio::spawn(async move { c2.checkpoint().await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!h.is_finished());
        tx.send(false).unwrap();
        assert!(h.await.unwrap().is_ok());

        tx.send(true).unwrap();
        let c3 = c.clone();
        let h = tokio::spawn(async move { c3.checkpoint().await });
        c.cancel.cancel();
        assert_eq!(h.await.unwrap(), Err(DownloadError::Cancelled));
    }

    #[tokio::test]
    async fn pool_prefers_order_spills_over_and_skips_cooling() {
        let (c, _tx) = ctl();
        let pool = MirrorPool::new(&[Mirror::OsuDirect, Mirror::Catboy]);
        let none = HashSet::new();
        let mut held = Vec::new();
        for _ in 0..Mirror::OsuDirect.max_parallel() {
            let (m, p) = pool.acquire(&none, &c).await.unwrap().unwrap();
            assert_eq!(m, Mirror::OsuDirect);
            held.push(p);
        }
        // osu.direct is full: next goes to catboy
        let (m, _p) = pool.acquire(&none, &c).await.unwrap().unwrap();
        assert_eq!(m, Mirror::Catboy);
        held.clear();
        // cooling mirror is skipped even with free slots
        pool.cool_down(Mirror::OsuDirect, Duration::from_secs(30));
        let (m, _p2) = pool.acquire(&none, &c).await.unwrap().unwrap();
        assert_eq!(m, Mirror::Catboy);
        // all excluded → None
        let all: HashSet<Mirror> = [Mirror::OsuDirect, Mirror::Catboy].into();
        assert!(pool.acquire(&all, &c).await.unwrap().is_none());
        assert_eq!(pool.capacity(), 10);
    }

    #[test]
    fn try_acquire_never_waits() {
        let pool = MirrorPool::new(&[Mirror::Nerinyan]);
        let none = HashSet::new();
        let held: Vec<_> = (0..Mirror::Nerinyan.max_parallel())
            .map(|_| pool.try_acquire(&none).unwrap())
            .collect();
        assert!(pool.try_acquire(&none).is_none());
        drop(held);
        assert!(pool.try_acquire(&[Mirror::Nerinyan].into()).is_none());
    }

    fn running(done: u64, total: u64, window_start: u64, age: Duration) -> (Running, Instant) {
        let live = Arc::new(Live::default());
        live.done.store(done, Ordering::Relaxed);
        live.total.store(total, Ordering::Relaxed);
        let now = Instant::now() + age;
        let r = Running {
            live,
            cancel: CancellationToken::new(),
            seen: 0,
            mark: (now - age, window_start),
        };
        (r, now)
    }

    #[test]
    fn slow_means_crawling_with_a_lot_left() {
        let w = HEDGE_WINDOW;
        // 65 KB/s with 10 MB to go: race it
        let (r, now) = running(650 * 1024, 11 << 20, 0, w);
        assert!(is_slow(&r, now));
        // stalled, size unknown
        let (r, now) = running(0, 0, 0, w);
        assert!(is_slow(&r, now));
        // catboy-like 500 KB/s is fine
        let (r, now) = running(5000 * 1024, 30 << 20, 0, w);
        assert!(!is_slow(&r, now));
        // slow but almost done
        let (r, now) = running(1 << 20, (1 << 20) + 100 * 1024, 0, w);
        assert!(!is_slow(&r, now));
    }

    /// Live download through the real mirrors; `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_download_small_set() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("929973.osz");
        let (c, _tx) = ctl();
        let client = crate::core::http_client();
        let pool = MirrorPool::new(&Mirror::ALL);
        download_set(&client, 929973, &pool, &dest, &c, |_| {}, |_, _| {})
            .await
            .unwrap();
        assert!(trim::osu_hashes(&dest).unwrap().len() >= 2);
        for m in [Mirror::Catboy, Mirror::Sayobot] {
            let one = MirrorPool::new(&[m]);
            let d = dir.path().join(format!("{m:?}.osz"));
            download_set(&client, 929973, &one, &d, &c, |_| {}, |_, _| {})
                .await
                .unwrap();
            assert!(trim::osu_hashes(&d).unwrap().len() >= 2, "{m:?}");
        }
        let err = download_set(
            &client,
            999_999_999,
            &MirrorPool::new(&[Mirror::Catboy, Mirror::OsuDirect]),
            &dest,
            &c,
            |_| {},
            |_, _| {},
        )
        .await;
        assert!(err.is_err());
    }
}
