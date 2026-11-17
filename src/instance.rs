// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Single-instance lock for `<home>` and cleanup of orphaned Chrome processes.
//!
//! A process takes the lock before it starts Chrome and keeps it until it exits. The operating
//! system releases the lock when the process stops, also after a crash. Thus, while a process
//! holds the lock, a Chrome process on `<home>/chrome-profile` that another process started is orphaned.

use anyhow::Result;
use std::ffi::OsString;
use std::fs::File;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::config;
use crate::platform::{self, MatchedProcess};

/// Open lock file that this process holds, or `None` before the first successful lock.
static INSTANCE_LOCK: Mutex<Option<File>> = Mutex::new(None);

/// Time that orphaned Chrome processes get to stop normally before a forced stop.
const ORPHAN_STOP_TIMEOUT: Duration = Duration::from_secs(3);

/// Time to wait for orphaned Chrome processes to disappear after a forced stop.
const ORPHAN_KILL_TIMEOUT: Duration = Duration::from_secs(1);

/// Interval between process checks while orphaned Chrome processes stop.
const ORPHAN_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Takes the `<home>` lock for this process, and returns `false` when another process holds it.
pub(crate) fn try_acquire_instance_lock() -> Result<bool> {
    let mut guard = INSTANCE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    if guard.is_some() {
        return Ok(true);
    }
    platform::create_private_dir_all(&config::home_dir())?;
    let file = platform::open_private_file(&config::instance_lock_path())?;
    match fs4::FileExt::try_lock(&file) {
        Ok(()) => {
            *guard = Some(file);
            Ok(true)
        }
        Err(fs4::TryLockError::WouldBlock) => Ok(false),
        Err(fs4::TryLockError::Error(err)) => Err(err.into()),
    }
}

/// Takes the `<home>` lock for this process, or fails when another process holds it.
pub(crate) fn acquire_instance_lock() -> Result<()> {
    if try_acquire_instance_lock()? {
        return Ok(());
    }
    anyhow::bail!(
        "Another flow-music-mcp process uses {}. Stop that process first, or use the music_login tool in it.",
        config::home_dir().display()
    )
}

/// Returns the `--user-data-dir` argument that Chrome processes on `profile_dir` have.
fn user_data_dir_arg(profile_dir: &Path) -> OsString {
    let mut arg = OsString::from("--user-data-dir=");
    arg.push(profile_dir.as_os_str());
    arg
}

/// Returns the top-level Chrome processes on `profile_dir`, separated into orphans and children of this process.
fn profile_chrome_processes(profile_dir: &Path) -> (Vec<MatchedProcess>, Vec<MatchedProcess>) {
    let own_pid = std::process::id();
    platform::top_level_processes_with_arg(&user_data_dir_arg(profile_dir))
        .into_iter()
        .partition(|p| p.parent != Some(own_pid))
}

/// Returns the process IDs of orphaned Chrome processes on `profile_dir`.
///
/// Call this function only while this process holds the `<home>` lock.
pub(crate) fn orphan_chrome_pids(profile_dir: &Path) -> Vec<u32> {
    profile_chrome_processes(profile_dir)
        .0
        .iter()
        .map(|p| p.pid)
        .collect()
}

/// Waits up to `limit` until none of `pids` exists, and returns the process IDs that still exist.
async fn wait_processes_gone(pids: &[u32], limit: Duration) -> Vec<u32> {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        let alive: Vec<u32> = pids
            .iter()
            .copied()
            .filter(|&pid| platform::process_exists(pid))
            .collect();
        if alive.is_empty() || tokio::time::Instant::now() >= deadline {
            return alive;
        }
        tokio::time::sleep(ORPHAN_POLL_INTERVAL).await;
    }
}

/// Stops orphaned Chrome processes on `profile_dir`: first normally, then by force.
///
/// A process that refuses the normal stop request (for example a window-less Chrome on Windows) stops by force at once.
///
/// Fails when a Chrome child of this process still uses `profile_dir`.
/// Call this function only while this process holds the `<home>` lock.
pub(crate) async fn stop_orphan_chrome(profile_dir: &Path) -> Result<()> {
    let (orphans, own) = profile_chrome_processes(profile_dir);
    if let Some(child) = own.first() {
        anyhow::bail!(
            "Chrome PID {} of this flow-music-mcp process already uses the profile at {}",
            child.pid,
            profile_dir.display()
        );
    }
    let pids: Vec<u32> = orphans.iter().map(|p| p.pid).collect();
    if pids.is_empty() {
        return Ok(());
    }
    let (asked, refused): (Vec<u32>, Vec<u32>) = pids
        .iter()
        .copied()
        .partition(|&pid| platform::interrupt_process(pid));
    let mut forced = refused;
    forced.extend(wait_processes_gone(&asked, ORPHAN_STOP_TIMEOUT).await);
    for &pid in &forced {
        platform::kill_process(pid);
    }
    let _ = wait_processes_gone(&forced, ORPHAN_KILL_TIMEOUT).await;
    Ok(())
}
