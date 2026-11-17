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

/// Default lower bound of the pause between queue jobs, in milliseconds.
pub(crate) const DEFAULT_PAUSE_MIN_MS: u64 = 5_000;

/// Default upper bound of the pause between queue jobs, in milliseconds.
pub(crate) const DEFAULT_PAUSE_MAX_MS: u64 = 15_000;

/// Runtime settings that the command line or its environment variables supply.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    /// Data root from `--home` or `FLOW_MUSIC_MCP_HOME`.
    pub(crate) home: Option<PathBuf>,
    /// Chrome or Chromium binary from `--chrome` or `FLOW_MUSIC_MCP_CHROME`.
    pub(crate) chrome: Option<PathBuf>,
    /// Fixed Chrome CDP port from `--cdp-port` or `FLOW_MUSIC_MCP_CDP_PORT`.
    pub(crate) cdp_port: Option<u16>,
    /// Inclusive pause bounds between queue jobs in milliseconds, from `--pause` or `FLOW_MUSIC_MCP_PAUSE`.
    pub(crate) pause_ms: (u64, u64),
}

impl Default for Settings {
    /// Returns settings with no overrides and the default pause bounds.
    fn default() -> Self {
        Self {
            home: None,
            chrome: None,
            cdp_port: None,
            pause_ms: (DEFAULT_PAUSE_MIN_MS, DEFAULT_PAUSE_MAX_MS),
        }
    }
}

/// Process-wide settings that `init_settings` stores once at startup.
static SETTINGS: OnceLock<Settings> = OnceLock::new();

/// Stores the parsed runtime settings once at startup.
pub(crate) fn init_settings(settings: Settings) {
    let _ = SETTINGS.set(settings);
}

/// Returns the stored runtime settings, or the defaults before `init_settings` runs.
fn settings() -> Settings {
    SETTINGS.get().cloned().unwrap_or_default()
}

/// Returns the `--chrome` binary path override if set at startup.
pub(crate) fn cli_chrome_path() -> Option<PathBuf> {
    settings().chrome
}

/// Returns the `flow-music-mcp` data root (`--home` or `FLOW_MUSIC_MCP_HOME`, then `~/.flow-music-mcp`).
pub(crate) fn home_dir() -> PathBuf {
    if let Some(v) = settings().home {
        return v;
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

/// Returns `<home_dir>/flow-music-mcp.lock`, which a process locks while it can start Chrome.
pub(crate) fn instance_lock_path() -> PathBuf {
    home_dir().join("flow-music-mcp.lock")
}

/// Returns `<home_dir>/chrome-profile` for the persistent `0700` Chrome user-data directory.
pub(crate) fn chrome_profile_dir() -> PathBuf {
    home_dir().join("chrome-profile")
}

/// Returns `true` when persisted session data exists in `<home_dir>/chrome-profile`.
pub(crate) fn has_persisted_session() -> bool {
    chrome_profile_dir().join("Default").is_dir()
}

/// Returns the fixed Chrome CDP port from `--cdp-port` or `FLOW_MUSIC_MCP_CDP_PORT`.
pub(crate) fn configured_cdp_port() -> Option<u16> {
    settings().cdp_port
}

/// Returns the inclusive pause bounds between queue jobs in milliseconds.
pub(crate) fn pause_bounds_ms() -> (u64, u64) {
    settings().pause_ms
}
