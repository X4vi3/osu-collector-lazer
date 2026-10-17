//! Beatmapset downloads from public mirrors.
//!
//! Mirrors throttle per connection (catboy ≈ 0.5 MB/s each) and per IP (osu.direct: 120 req/min,
//! burst 25/5 s), so all enabled mirrors are used at once, each with its own parallel cap.
//! A rate-limited mirror cools down while other mirrors keep downloading.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
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

    /// First mirror in preference order that is not excluded, not cooling down and has a free slot.
    /// Waits while all candidates are busy. `None` when every mirror is excluded.
    async fn acquire(
        &self,
        exclude: &HashSet<Mirror>,
        ctl: &Control,
    ) -> Result<Option<(Mirror, OwnedSemaphorePermit)>, DownloadError> {
        loop {
            let mut any = false;
            let now = Instant::now();
            for s in &self.slots {
                if exclude.contains(&s.mirror) {
                    continue;
                }
                any = true;
                if *s.cooldown_until.lock().unwrap() > now {
                    continue;
                }
                if let Ok(p) = s.sem.clone().try_acquire_owned() {
                    return Ok(Some((s.mirror, p)));
                }
            }
            if !any {
                return Ok(None);
            }
            ctl.sleep(Duration::from_millis(150)).await?;
        }
    }
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

/// Downloads `set_id` into `dest` using whichever mirror is free, falling back across mirrors.
/// Calls `progress(done, total)` while streaming. Returns the mirror that succeeded.
pub async fn download_set(
    client: &reqwest::Client,
    set_id: u64,
    pool: &MirrorPool,
    dest: &Path,
    ctl: &Control,
    mut on_mirror: impl FnMut(Mirror),
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<Mirror, DownloadError> {
    let mut exhausted: HashSet<Mirror> = HashSet::new();
    let mut attempts: HashMap<Mirror, u32> = HashMap::new();
    let mut outcomes: HashMap<Mirror, Outcome> = HashMap::new();
    let mut last: Option<DownloadError> = None;

    loop {
        ctl.checkpoint().await?;
        let Some((mirror, permit)) = pool.acquire(&exhausted, ctl).await? else {
            break;
        };
        on_mirror(mirror);
        let res = try_once(client, mirror.url(set_id), dest, ctl, &mut progress).await;
        drop(permit);
        let n = attempts.entry(mirror).or_default();
        *n += 1;
        match res? {
            Attempt::Ok => return Ok(mirror),
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
                } else {
                    ctl.sleep(Duration::from_secs(1 << *n)).await?;
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
    dest: &Path,
    ctl: &Control,
    progress: &mut impl FnMut(u64, Option<u64>),
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

    let total = resp.content_length();
    let part = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| DownloadError::Failed {
            mirror: "-",
            reason: crate::trf!("не удалось создать файл: {e}", "could not create file: {e}"),
        })?;
    let mut done = 0u64;
    progress(0, total);
    let mut stream = resp.bytes_stream();
    loop {
        if let Err(e) = ctl.checkpoint().await {
            drop(file);
            let _ = tokio::fs::remove_file(&part).await;
            return Err(e);
        }
        let chunk = tokio::select! {
            _ = ctl.cancel.cancelled() => None,
            c = stream.next() => Some(c),
        };
        let Some(chunk) = chunk else {
            drop(file);
            let _ = tokio::fs::remove_file(&part).await;
            return Err(DownloadError::Cancelled);
        };
        match chunk {
            None => break,
            Some(Ok(bytes)) => {
                if let Err(e) = file.write_all(&bytes).await {
                    drop(file);
                    let _ = tokio::fs::remove_file(&part).await;
                    return Err(DownloadError::Failed {
                        mirror: "-",
                        reason: crate::trf!("запись на диск: {e}", "disk write: {e}"),
                    });
                }
                done += bytes.len() as u64;
                progress(done, total);
            }
            Some(Err(e)) => {
                drop(file);
                let _ = tokio::fs::remove_file(&part).await;
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

    let check = part.clone();
    let valid = tokio::task::spawn_blocking(move || trim::osu_hashes(&check)).await;
    match valid {
        Ok(Ok(_)) => {
            tokio::fs::rename(&part, dest)
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
            let _ = tokio::fs::remove_file(&part).await;
            Ok(Attempt::NextMirror {
                reason: crate::trf!("плохой архив: {e}", "bad archive: {e}"),
                not_found: false,
            })
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
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
