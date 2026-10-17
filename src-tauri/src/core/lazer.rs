//! Talking to osu!lazer only through its own mechanisms: `osu!.exe <files>` IPC import.
//!
//! When lazer is already running, a second `osu!.exe a.osz b.osz` forwards each path over the
//! `osu-lazer` named pipe and exits. When it is NOT running, that process becomes the game itself,
//! so imports are only sent once the pipe exists.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

/// Named pipe osu-framework creates for lazer's IPC host (`osu-framework-` + IPC_PIPE_NAME).
const PIPE_NAME: &str = "osu-framework-osu-lazer";
/// Keep the command line well under Windows' 32767-char limit.
const MAX_CMDLINE: usize = 30_000;
pub const MAX_BATCH: usize = 20;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;

pub fn candidate_paths() -> Vec<PathBuf> {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return vec![];
    };
    vec![
        local.join("osulazer").join("current").join("osu!.exe"), // Velopack
        local.join("osulazer").join("osu!.exe"),                 // legacy Squirrel
    ]
}

/// The configured path if valid, otherwise the first autodetected one.
pub fn resolve_exe(configured: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = configured {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    candidate_paths().into_iter().find(|p| p.is_file())
}

/// True once lazer's IPC server is listening. Enumerates `\\.\pipe\` instead of opening the pipe,
/// so lazer never sees a stray connection.
pub fn ipc_ready() -> bool {
    #[cfg(windows)]
    {
        match std::fs::read_dir(r"\\.\pipe\") {
            Ok(entries) => entries.flatten().any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(PIPE_NAME)
            }),
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LazerStatus {
    pub found: bool,
    pub path: Option<String>,
    /// Running with IPC ready to accept imports
    pub running: bool,
}

pub fn status(configured: Option<&str>) -> LazerStatus {
    let exe = resolve_exe(configured);
    LazerStatus {
        found: exe.is_some(),
        path: exe.map(|p| p.display().to_string()),
        running: ipc_ready(),
    }
}

/// Starts lazer detached (it outlives this app) and waits until IPC is ready.
pub async fn launch_and_wait(exe: &Path, timeout: Duration) -> Result<(), String> {
    if ipc_ready() {
        return Ok(());
    }
    let mut cmd = std::process::Command::new(exe);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(DETACHED_PROCESS);
    }
    cmd.spawn().map_err(|e| {
        crate::trf!(
            "не удалось запустить osu!lazer: {e}",
            "could not start osu!lazer: {e}"
        )
    })?;
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if ipc_ready() {
            // pipe binds early in startup; give the game a moment to register its import channel
            tokio::time::sleep(Duration::from_secs(3)).await;
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(crate::core::i18n::tr(
        "osu!lazer не запустился вовремя",
        "osu!lazer did not start in time",
    )
    .into())
}

/// Splits paths into batches of at most `MAX_BATCH` files and `MAX_CMDLINE` characters.
pub fn batches(paths: &[PathBuf]) -> Vec<Vec<PathBuf>> {
    let mut out: Vec<Vec<PathBuf>> = Vec::new();
    let mut cur: Vec<PathBuf> = Vec::new();
    let mut len = 0usize;
    for p in paths {
        let l = p.as_os_str().len() + 3;
        if !cur.is_empty() && (cur.len() >= MAX_BATCH || len + l > MAX_CMDLINE) {
            out.push(std::mem::take(&mut cur));
            len = 0;
        }
        len += l;
        cur.push(p.clone());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ImportError {
    #[error(
        "{}",
        crate::core::i18n::tr("osu!lazer не запущен", "osu!lazer is not running")
    )]
    NotRunning,
    #[error(
        "{}",
        crate::core::i18n::tr(
            "osu!lazer не ответил вовремя (IPC timeout)",
            "osu!lazer did not respond in time (IPC timeout)"
        )
    )]
    Timeout,
    #[error("{p}: {0}", p = crate::core::i18n::tr("ошибка вызова osu!.exe", "osu!.exe call failed"))]
    Failed(String),
}

/// Sends one batch to the running lazer. Each file gets up to 3 s for lazer to acknowledge.
pub async fn import_batch(exe: &Path, files: &[PathBuf]) -> Result<(), ImportError> {
    if !ipc_ready() {
        return Err(ImportError::NotRunning);
    }
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(files).kill_on_drop(true);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| ImportError::Failed(e.to_string()))?;
    let limit = Duration::from_secs(15 + 4 * files.len() as u64);
    match tokio::time::timeout(limit, child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(if ipc_ready() {
            ImportError::Timeout
        } else {
            ImportError::Failed(crate::trf!("код выхода {status}", "exit code {status}"))
        }),
        Ok(Err(e)) => Err(ImportError::Failed(e.to_string())),
        Err(_) => {
            let _ = child.kill().await;
            Err(ImportError::Timeout)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_respect_count_and_length() {
        let short: Vec<PathBuf> = (0..45)
            .map(|i| PathBuf::from(format!("C:\\t\\{i}.osz")))
            .collect();
        let b = batches(&short);
        assert_eq!(b.iter().map(Vec::len).collect::<Vec<_>>(), vec![20, 20, 5]);

        let long: Vec<PathBuf> = (0..10)
            .map(|i| PathBuf::from(format!("C:\\{}\\{i}.osz", "x".repeat(9000))))
            .collect();
        let b = batches(&long);
        assert!(b.iter().all(
            |batch| batch.iter().map(|p| p.as_os_str().len() + 3).sum::<usize>() <= MAX_CMDLINE
        ));
        assert_eq!(b.iter().map(Vec::len).sum::<usize>(), 10);
        assert!(batches(&[]).is_empty());
    }

    #[test]
    fn configured_path_must_exist() {
        assert_eq!(resolve_exe(Some("Z:\\definitely\\missing\\osu!.exe")), None);
    }

    #[test]
    fn pipe_enumeration_works() {
        // must not panic or error regardless of whether lazer runs
        let _ = ipc_ready();
        #[cfg(windows)]
        assert!(std::fs::read_dir(r"\\.\pipe\").is_ok());
    }
}
