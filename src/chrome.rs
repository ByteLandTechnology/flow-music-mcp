// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Chrome binary discovery, persistent profile management, and login inspection.

use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::auth::is_music_session_cookie;
use crate::cdp::{CdpConnection, fetch_json_version};
use crate::config::{chrome_profile_dir, cli_chrome_path, configured_cdp_port};
use crate::private_fs::create_private_dir_all;

/// Owned Chrome child process backed by the persistent `0700` user-data directory.
pub(crate) struct ManagedChrome {
    /// Running headless Chrome process handle.
    child: tokio::process::Child,
    /// Persistent `0700` Chrome profile directory.
    profile_dir: PathBuf,
    /// Local HTTP CDP discovery endpoint (`http://127.0.0.1:<port>`).
    pub(crate) http_endpoint: String,
    /// Chrome `User-Agent` string reported by `/json/version`, if non-empty.
    pub(crate) user_agent: String,
}

impl Drop for ManagedChrome {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// Interactive Chrome sign-in process without CDP debugging flags.
pub(crate) struct LoginChrome {
    /// Running visible Chrome process handle.
    child: tokio::process::Child,
    /// Persistent `0700` Chrome profile directory.
    pub(crate) profile_dir: PathBuf,
    /// Timestamp when the interactive login window launched.
    pub(crate) started_at: SystemTime,
}

impl Drop for LoginChrome {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// Sign-in state read from scratch copies of a Chrome user profile directory.
#[derive(Debug, Default)]
pub(crate) struct LoginProfileProbe {
    /// Google Accounts rejection URL observed in profile history.
    pub(crate) rejected_url: Option<String>,
    /// True when the Google Flow Music session cookie was written since launch.
    pub(crate) has_session_cookie: bool,
}

/// Sets child-process English locale environment variables when unset in the parent environment.
fn apply_english_env(cmd: &mut tokio::process::Command) {
    if std::env::var_os("LANGUAGE").is_none() {
        cmd.env("LANGUAGE", "en_US:en");
    }
    if std::env::var_os("LANG").is_none() {
        cmd.env("LANG", "en_US.UTF-8");
    }
}

/// Finds and validates the Chrome or Chromium binary path on disk.
fn resolve_chrome_binary() -> Result<PathBuf> {
    let chrome = find_chrome().ok_or_else(|| {
        anyhow::anyhow!(
            "Failed to find Chrome or Chromium. Install Chrome, pass --chrome, or set FLOW_MUSIC_MCP_CHROME."
        )
    })?;
    if !chrome.is_file() {
        anyhow::bail!(
            "Configured Chrome binary does not exist: {}",
            chrome.display()
        );
    }
    Ok(chrome)
}

/// Waits up to `limit` for Chrome to remove `SingletonLock` on exit.
async fn wait_profile_unlocked(profile_dir: &Path, limit: Duration) {
    let lock = profile_dir.join("SingletonLock");
    let deadline = tokio::time::Instant::now() + limit;
    while std::fs::symlink_metadata(&lock).is_ok() {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Removes Chrome singleton lock files from `profile_dir`.
#[cfg(unix)]
fn remove_singleton_artifacts(profile_dir: &Path) {
    for name in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
        let _ = std::fs::remove_file(profile_dir.join(name));
    }
}

/// Ensures `profile_dir` exists with `0700` permissions and clears stale lock files.
async fn prepare_profile_dir(profile_dir: &Path) -> Result<()> {
    create_private_dir_all(profile_dir)?;
    let _ = std::fs::remove_file(profile_dir.join("DevToolsActivePort"));

    #[cfg(unix)]
    {
        let lock_path = profile_dir.join("SingletonLock");
        if std::fs::symlink_metadata(&lock_path).is_ok() {
            let parsed_pid = std::fs::read_link(&lock_path).ok().and_then(|target| {
                target
                    .to_string_lossy()
                    .rsplit('-')
                    .next()
                    .and_then(|s| s.parse::<u32>().ok())
                    .filter(|&p| p > 0)
            });

            let Some(pid) = parsed_pid else {
                remove_singleton_artifacts(profile_dir);
                return Ok(());
            };

            let ps_out = tokio::process::Command::new("/bin/ps")
                .arg("-p")
                .arg(pid.to_string())
                .arg("-o")
                .arg("ppid=,args=")
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .await;

            let Ok(output) = ps_out else {
                remove_singleton_artifacts(profile_dir);
                return Ok(());
            };
            if !output.status.success() {
                remove_singleton_artifacts(profile_dir);
                return Ok(());
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let trimmed = stdout.trim();
            if trimmed.is_empty() {
                remove_singleton_artifacts(profile_dir);
                return Ok(());
            }

            let profile_str = profile_dir.to_string_lossy();
            let Some((ppid_str, args_str)) = trimmed.split_once(char::is_whitespace) else {
                remove_singleton_artifacts(profile_dir);
                return Ok(());
            };
            if !args_str.contains(profile_str.as_ref()) {
                remove_singleton_artifacts(profile_dir);
                return Ok(());
            }

            let ppid = ppid_str.trim().parse::<u32>().unwrap_or(0);
            if ppid <= 1 {
                let _ = tokio::process::Command::new("/bin/kill")
                    .arg("-INT")
                    .arg(pid.to_string())
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await;
                wait_profile_unlocked(profile_dir, Duration::from_secs(3)).await;
                if std::fs::symlink_metadata(&lock_path).is_ok() {
                    let _ = tokio::process::Command::new("/bin/kill")
                        .arg("-KILL")
                        .arg(pid.to_string())
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                        .await;
                    wait_profile_unlocked(profile_dir, Duration::from_secs(1)).await;
                }
                remove_singleton_artifacts(profile_dir);
                let _ = std::fs::remove_file(profile_dir.join("DevToolsActivePort"));
            } else {
                anyhow::bail!(
                    "Chrome profile at {} is already in use by another running flow-music-mcp process (Chrome PID {})",
                    profile_dir.display(),
                    pid
                );
            }
        }
    }

    Ok(())
}

/// Sends an interrupt signal to `child`, waits up to `wait_limit`, and force-kills if needed.
async fn stop_child_gracefully(child: &mut tokio::process::Child, wait_limit: Duration) {
    if !matches!(child.try_wait(), Ok(None)) {
        return;
    }

    #[cfg(unix)]
    if let Some(pid) = child.id() {
        let _ = tokio::process::Command::new("/bin/kill")
            .arg("-INT")
            .arg(pid.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        let _ = tokio::time::timeout(wait_limit, child.wait()).await;
    }

    #[cfg(not(unix))]
    let _ = wait_limit;

    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

/// Activates the Chrome window for process `pid` so the user sees it during sign-in.
fn focus_chrome_process(pid: u32) {
    #[cfg(target_os = "macos")]
    {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let script = format!(
                "ObjC.import('AppKit'); const a = $.NSRunningApplication.runningApplicationWithProcessIdentifier({pid}); if (!a.isNil()) a.activateWithOptions(3);"
            );
            let _ = tokio::process::Command::new("/usr/bin/osascript")
                .arg("-l")
                .arg("JavaScript")
                .arg("-e")
                .arg(script)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = pid;
}

impl ManagedChrome {
    /// Launches headless CDP-enabled Chrome on the persistent `0700` profile directory and waits for its CDP HTTP endpoint.
    pub(crate) async fn launch(start_url: &str) -> Result<Self> {
        let chrome = resolve_chrome_binary()?;
        let profile_dir = chrome_profile_dir();
        prepare_profile_dir(&profile_dir).await?;

        let explicit_port = configured_cdp_port();
        let launch_port = explicit_port.unwrap_or(0);

        let initial_url = if start_url.trim().is_empty() {
            "about:blank"
        } else {
            start_url
        };

        let mut cmd = tokio::process::Command::new(&chrome);
        apply_english_env(&mut cmd);
        cmd.arg(format!("--remote-debugging-port={}", launch_port))
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg("--headless=new")
            .arg("--lang=en-US")
            .arg("--accept-lang=en-US,en")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--hide-crash-restore-bubble")
            .arg("--window-size=1280,860")
            .arg("--disable-backgrounding-occluded-windows")
            .arg("--disable-renderer-backgrounding")
            .arg("--disable-background-timer-throttling");

        cmd.arg(initial_url)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("Failed to launch Chrome at {}", chrome.display()))?;

        let active_port_file = profile_dir.join("DevToolsActivePort");
        let mut discovered: Option<(String, String)> = None;

        for _ in 0..120 {
            if let Ok(Some(status)) = child.try_wait() {
                let _ = tokio::fs::remove_file(&active_port_file).await;
                anyhow::bail!("Chrome exited early with status {}", status);
            }

            let port = if let Some(p) = explicit_port {
                Some(p)
            } else {
                read_active_cdp_port(&active_port_file).await
            };

            if let Some(port) = port {
                let endpoint = format!("http://127.0.0.1:{}", port);
                if let Some(version) = fetch_json_version(&endpoint).await {
                    let ua = version
                        .get("User-Agent")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .unwrap_or("")
                        .to_string();
                    discovered = Some((endpoint, ua));
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(125)).await;
        }

        let Some((http_endpoint, user_agent)) = discovered else {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = tokio::fs::remove_file(&active_port_file).await;
            let port_label =
                explicit_port.map_or_else(|| "ephemeral (0)".to_string(), |p| p.to_string());
            anyhow::bail!(
                "Timed out waiting for Chrome CDP endpoint (port {}) after launching {}",
                port_label,
                chrome.display()
            );
        };

        Ok(Self {
            child,
            profile_dir,
            http_endpoint,
            user_agent,
        })
    }

    /// Gracefully stops Chrome so the persistent profile directory and cookie store flush to disk.
    pub(crate) async fn shutdown(mut self) {
        if let Some(version) = fetch_json_version(&self.http_endpoint).await
            && let Some(ws) = version.get("webSocketDebuggerUrl").and_then(|v| v.as_str())
            && let Ok(conn) = CdpConnection::connect_basic(ws).await
        {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                conn.call("Browser.close", json!({})),
            )
            .await;
        }

        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        if matches!(self.child.try_wait(), Ok(None)) {
            stop_child_gracefully(&mut self.child, Duration::from_secs(3)).await;
        }

        wait_profile_unlocked(&self.profile_dir, Duration::from_secs(2)).await;
        let _ = tokio::fs::remove_file(self.profile_dir.join("DevToolsActivePort")).await;
    }
}

impl LoginChrome {
    /// Launches standard visible Chrome on a clean `<home>/chrome-profile` (`0700`) without CDP flags.
    pub(crate) async fn launch(start_url: &str) -> Result<Self> {
        let chrome = resolve_chrome_binary()?;
        let profile_dir = chrome_profile_dir();
        if profile_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&profile_dir).await;
        }
        prepare_profile_dir(&profile_dir).await?;

        let initial_url = if start_url.trim().is_empty() {
            "about:blank"
        } else {
            start_url
        };
        let started_at = SystemTime::now();

        let mut cmd = tokio::process::Command::new(&chrome);
        apply_english_env(&mut cmd);
        cmd.arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg("--lang=en-US")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg(initial_url)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        let child = cmd
            .spawn()
            .with_context(|| format!("Failed to launch Chrome at {}", chrome.display()))?;

        if let Some(pid) = child.id() {
            focus_chrome_process(pid);
        }

        Ok(Self {
            child,
            profile_dir,
            started_at,
        })
    }

    /// Returns `true` when the interactive Chrome child process has exited.
    pub(crate) fn try_exited(&mut self) -> Result<bool> {
        Ok(self.child.try_wait()?.is_some())
    }

    /// Gracefully stops the interactive Chrome process while keeping `<home>/chrome-profile` on disk.
    pub(crate) async fn shutdown(mut self) {
        stop_child_gracefully(&mut self.child, Duration::from_secs(5)).await;
        wait_profile_unlocked(&self.profile_dir, Duration::from_secs(2)).await;
    }
}

/// Copies a SQLite database file and its `-wal`/`-journal` sidecars into `scratch_dir`.
fn copy_sqlite_files_to_scratch(
    src: &Path,
    scratch_dir: &Path,
    base_name: &str,
) -> Option<PathBuf> {
    if !src.is_file() {
        return None;
    }
    let dst = scratch_dir.join(base_name);
    std::fs::copy(src, &dst).ok()?;
    let parent = src.parent()?;
    let src_name = src.file_name()?.to_string_lossy();
    for suffix in ["-wal", "-journal"] {
        let sidecar_src = parent.join(format!("{src_name}{suffix}"));
        if sidecar_src.is_file() {
            let sidecar_dst = scratch_dir.join(format!("{base_name}{suffix}"));
            let _ = std::fs::copy(sidecar_src, sidecar_dst);
        }
    }
    Some(dst)
}

/// Inspects a scratch copy of Chrome's `History` SQLite database for Google Accounts rejection URLs.
fn probe_history_db(scratch_history: &Path, min_webkit_us: i64, probe: &mut LoginProfileProbe) {
    let Ok(conn) = rusqlite::Connection::open(scratch_history) else {
        return;
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT url FROM urls WHERE last_visit_time >= ?1 ORDER BY last_visit_time DESC, id DESC LIMIT 25",
    ) else {
        return;
    };
    let Ok(rows) = stmt.query_map([min_webkit_us], |row| row.get::<_, String>(0)) else {
        return;
    };
    for url in rows.flatten() {
        if url.contains("accounts.google.com/v3/signin/rejected") {
            probe.rejected_url = Some(url);
            return;
        }
    }
}

/// Returns `true` when a scratch copy of Chrome's `Cookies` database holds the Google Flow Music session cookie.
fn probe_cookies_db(scratch_cookies: &Path, min_webkit_us: i64) -> bool {
    let Ok(conn) = rusqlite::Connection::open(scratch_cookies) else {
        return false;
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT host_key, name FROM cookies WHERE last_update_utc >= ?1")
    else {
        return false;
    };
    let Ok(rows) = stmt.query_map([min_webkit_us], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return false;
    };
    for pair in rows.flatten() {
        if is_music_session_cookie(&pair.0, &pair.1) {
            return true;
        }
    }
    false
}

/// Inspects Chrome `History` and `Cookies` files for entries updated since `started_at`.
pub(crate) async fn inspect_login_profile(
    profile_dir: &Path,
    started_at: SystemTime,
) -> LoginProfileProbe {
    const WEBKIT_EPOCH_OFFSET_SECS: i64 = 11_644_473_600;
    let unix_secs = started_at
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let min_webkit_us = (unix_secs.saturating_sub(2) + WEBKIT_EPOCH_OFFSET_SECS) * 1_000_000;

    let dir = profile_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut probe = LoginProfileProbe::default();

        let Ok(scratch) = tempfile::tempdir() else {
            return probe;
        };

        let history_src = dir.join("Default").join("History");
        if let Some(scratch_history) =
            copy_sqlite_files_to_scratch(&history_src, scratch.path(), "History")
        {
            probe_history_db(&scratch_history, min_webkit_us, &mut probe);
        }

        let cookie_candidates = [
            dir.join("Default").join("Cookies"),
            dir.join("Default").join("Network").join("Cookies"),
        ];
        for (idx, src) in cookie_candidates.iter().enumerate() {
            let base = format!("Cookies_{idx}");
            if let Some(scratch_cookies) = copy_sqlite_files_to_scratch(src, scratch.path(), &base)
                && probe_cookies_db(&scratch_cookies, min_webkit_us)
            {
                probe.has_session_cookie = true;
                break;
            }
        }

        probe
    })
    .await
    .unwrap_or_default()
}

/// Reads the ephemeral CDP port number written to Chrome's `DevToolsActivePort` file.
async fn read_active_cdp_port(path: &Path) -> Option<u16> {
    let raw = tokio::fs::read_to_string(path).await.ok()?;
    let first_line = raw.lines().next()?.trim();
    first_line.parse::<u16>().ok().filter(|&p| p > 0)
}

/// Finds the Chrome or Chromium binary from `--chrome`, `FLOW_MUSIC_MCP_CHROME`, or OS defaults.
pub(crate) fn find_chrome() -> Option<PathBuf> {
    if let Some(p) = cli_chrome_path() {
        return Some(p);
    }
    if let Ok(p) = std::env::var("FLOW_MUSIC_MCP_CHROME") {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    chrome_candidates().into_iter().find(|p| p.is_file())
}

/// Lists OS-specific candidate paths for Chrome, Chromium, Brave, Edge, and Playwright builds.
fn chrome_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut out = vec![
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            PathBuf::from(
                "/Applications/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            ),
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
            PathBuf::from("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"),
            PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
        ];

        let mut cache_roots: Vec<PathBuf> = Vec::new();
        if let Some(h) = std::env::home_dir() {
            out.insert(
                1,
                h.join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            );
            cache_roots.push(h);
        }
        if let Ok(h) = std::env::var("HOME") {
            let hp = PathBuf::from(h);
            if !cache_roots.contains(&hp) {
                cache_roots.push(hp);
            }
        }
        if let Ok(u) = std::env::var("USER") {
            let up = PathBuf::from("/Users").join(u);
            if !cache_roots.contains(&up) {
                cache_roots.push(up);
            }
        }

        let mut pw_dirs: Vec<PathBuf> = Vec::new();
        let mut expanded_roots: Vec<PathBuf> = cache_roots.clone();
        for root in &cache_roots {
            if let Ok(rd) = std::fs::read_dir(root.join(".gemini")) {
                for entry in rd.flatten() {
                    let path = entry.path();
                    if path.is_dir() && !expanded_roots.contains(&path) {
                        expanded_roots.push(path);
                    }
                }
            }
        }
        for root in expanded_roots {
            let pw = root.join("Library/Caches/ms-playwright");
            if let Ok(rd) = std::fs::read_dir(&pw) {
                for entry in rd.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("chromium-")
                        && !name_str.starts_with("chromium_headless_shell-")
                    {
                        pw_dirs.push(entry.path());
                    }
                }
            }
        }
        pw_dirs.sort_by(|a, b| b.cmp(a));
        for dir in pw_dirs {
            out.push(dir.join(
                "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            ));
            out.push(dir.join(
                "chrome-mac/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            ));
            out.push(dir.join("chrome-mac-arm64/Chromium.app/Contents/MacOS/Chromium"));
            out.push(dir.join("chrome-mac/Chromium.app/Contents/MacOS/Chromium"));
        }
        out
    }
    #[cfg(target_os = "windows")]
    {
        let mut out = Vec::new();
        for env_key in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Ok(base) = std::env::var(env_key) {
                let root = PathBuf::from(base);
                out.push(root.join("Google/Chrome/Application/chrome.exe"));
                out.push(root.join("Chromium/Application/chrome.exe"));
                out.push(root.join("Microsoft/Edge/Application/msedge.exe"));
            }
        }
        out
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        vec![
            PathBuf::from("/usr/bin/google-chrome"),
            PathBuf::from("/usr/bin/chromium"),
            PathBuf::from("/usr/bin/chromium-browser"),
            PathBuf::from("/snap/bin/chromium"),
        ]
    }
}
