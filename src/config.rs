// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Runtime paths, Google Flow Music URL constants, and CLI override storage.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Google Flow Music application origin. All internal API calls use this origin.
pub(crate) const APP_ORIGIN: &str = "https://www.flowmusic.app";

/// Google Flow Music library page with the English interface parameter (`hl=en`).
pub(crate) const APP_URL: &str = "https://www.flowmusic.app/library?hl=en";

/// Google Flow Music start page for interactive sign-in with the English interface parameter.
pub(crate) const LOGIN_URL: &str = "https://www.flowmusic.app/?hl=en";

/// Process-wide override for `--home`.
static CLI_HOME_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Process-wide override for `--chrome`.
static CLI_CHROME_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Stores the `--home` override once at startup.
///
/// [`home_dir`] checks this value before environment variables and platform defaults.
pub(crate) fn init_cli_home(home: Option<PathBuf>) {
    if let Some(h) = home {
        let _ = CLI_HOME_DIR.set(h);
    }
}

/// Stores the `--chrome` executable override once at startup.
pub(crate) fn set_chrome_path(path: PathBuf) {
    let _ = CLI_CHROME_PATH.set(path);
}

/// Returns the `--chrome` binary path override if set at startup.
pub(crate) fn cli_chrome_path() -> Option<PathBuf> {
    CLI_CHROME_PATH.get().cloned()
}

/// Returns the `flow-music-mcp` data root (`--home`, then `FLOW_MUSIC_MCP_HOME`, then `~/.flow-music-mcp`).
pub(crate) fn home_dir() -> PathBuf {
    if let Some(v) = CLI_HOME_DIR.get() {
        return v.clone();
    }
    if let Ok(v) = std::env::var("FLOW_MUSIC_MCP_HOME")
        && !v.trim().is_empty()
    {
        return PathBuf::from(v);
    }
    std::env::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".flow-music-mcp")
}

/// Returns `<home_dir>/downloads` for all downloaded media files.
pub(crate) fn downloads_dir() -> PathBuf {
    home_dir().join("downloads")
}

/// Returns `<home_dir>/flow-music-mcp.db` for the local SQLite database.
pub(crate) fn db_path() -> PathBuf {
    home_dir().join("flow-music-mcp.db")
}

/// Returns `<home_dir>/chrome-profile` for the persistent `0700` Chrome user-data directory.
pub(crate) fn chrome_profile_dir() -> PathBuf {
    home_dir().join("chrome-profile")
}

/// Returns `true` when persisted session data exists in `<home_dir>/chrome-profile`.
pub(crate) fn has_persisted_session() -> bool {
    chrome_profile_dir().join("Default").is_dir()
}

/// Returns the explicit Chrome CDP port from `FLOW_MUSIC_MCP_CHROME_CDP_PORT` when valid and positive.
pub(crate) fn configured_cdp_port() -> Option<u16> {
    std::env::var("FLOW_MUSIC_MCP_CHROME_CDP_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|&p| p > 0)
}

/// Reads inter-job pause bounds in milliseconds from `FLOW_MUSIC_MCP_PAUSE_MIN_S`
/// and `FLOW_MUSIC_MCP_PAUSE_MAX_S`, defaulting to `5_000..=15_000`.
pub(crate) fn pause_bounds_ms() -> (u64, u64) {
    let min_ms = std::env::var("FLOW_MUSIC_MCP_PAUSE_MIN_S")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| s.is_finite())
        .map(|s| (s.clamp(0.0, 86_400.0) * 1000.0) as u64)
        .unwrap_or(5_000);
    let max_ms = std::env::var("FLOW_MUSIC_MCP_PAUSE_MAX_S")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| s.is_finite())
        .map(|s| ((s.clamp(0.0, 86_400.0) * 1000.0) as u64).max(min_ms))
        .unwrap_or(15_000.max(min_ms));
    (min_ms, max_ms)
}
