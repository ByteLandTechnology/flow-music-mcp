// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Chrome DevTools Protocol (CDP) browser engine backed by a persistent Chrome profile.
//!
//! The engine keeps one headless page on the Google Flow Music origin and runs the
//! embedded TypeScript runtime in that page. The runtime sends internal API requests
//! with the session of the page.

use anyhow::Result;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;
use url::Url;

use crate::cdp::{CdpConnection, find_or_create_page_target};
use crate::chrome::{LoginChrome, ManagedChrome, inspect_login_profile};
use crate::config::{APP_ORIGIN, APP_URL};

/// Browser engine identifier reported by status tools.
pub(crate) const ENGINE_NAME: &str = "chrome-cdp";

/// Compiled browser TypeScript runtime bundle exposing `window.__flowMusicMcp`.
const MUSIC_RUNTIME_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/music_runtime.js"));

/// Default macOS Chrome `User-Agent` fallback before Chrome launches.
#[cfg(target_os = "macos")]
const DEFAULT_CHROME_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36";

/// Default Windows Chrome `User-Agent` fallback before Chrome launches.
#[cfg(target_os = "windows")]
const DEFAULT_CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36";

/// Default Linux Chrome `User-Agent` fallback before Chrome launches.
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
const DEFAULT_CHROME_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36";

/// Active headless Chrome process paired with its CDP connection.
struct ActiveChrome {
    /// Owned headless Chrome process handle.
    managed: ManagedChrome,
    /// Open page CDP connection.
    cdp: Arc<CdpConnection>,
}

/// Poll status reported for an open standard interactive Chrome sign-in window.
pub(crate) struct LoginPollStatus {
    /// True when the user closed the interactive Chrome process.
    pub(crate) exited: bool,
    /// Google Accounts rejection URL observed during sign-in, if any.
    pub(crate) rejected_url: Option<String>,
    /// True when the Google Flow Music session cookie exists in the login profile.
    pub(crate) has_session_cookie: bool,
}

/// Manages one Chrome process and page CDP session per MCP server instance.
pub(crate) struct ChromeEngine {
    /// Active headless Chrome process and page CDP connection.
    inner: Mutex<Option<ActiveChrome>>,
    /// Active visible Chrome process during interactive sign-in.
    login_chrome: Mutex<Option<LoginChrome>>,
    /// Serializes Chrome process launches, shutdowns, and session promotions.
    ensure_lock: Mutex<()>,
    /// True while a visible interactive sign-in window is open.
    visible: AtomicBool,
    /// Cached Chrome `User-Agent` string observed from `/json/version`.
    user_agent: Mutex<Option<String>>,
}

impl ChromeEngine {
    /// Creates a new [`ChromeEngine`] controller with no active browser process.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(None),
            login_chrome: Mutex::new(None),
            ensure_lock: Mutex::new(()),
            visible: AtomicBool::new(false),
            user_agent: Mutex::new(None),
        })
    }

    /// Gracefully stops active Chrome processes while keeping `<home>/chrome-profile` on disk.
    pub(crate) async fn shutdown(&self) {
        let _guard = self.ensure_lock.lock().await;
        let active = self.inner.lock().await.take();
        let login = self.login_chrome.lock().await.take();
        if let Some(lc) = login {
            lc.shutdown().await;
        }
        if let Some(a) = active {
            shutdown_active(a).await;
        }
        self.visible.store(false, Ordering::Release);
    }

    /// Returns `true` when a standard interactive Chrome sign-in window is active.
    pub(crate) async fn has_login_chrome(&self) -> bool {
        self.login_chrome.lock().await.is_some()
    }

    /// Launches a clean visible Chrome window without CDP flags for interactive Google Sign-In.
    pub(crate) async fn launch_interactive_login(&self, start_url: &str) -> Result<()> {
        let _ensure = self.ensure_lock.lock().await;
        let prev_active = self.inner.lock().await.take();
        let prev_login = self.login_chrome.lock().await.take();
        if let Some(old) = prev_active {
            shutdown_active(old).await;
        }
        if let Some(old) = prev_login {
            old.shutdown().await;
        }
        self.visible.store(true, Ordering::Release);
        let login = LoginChrome::launch(start_url).await?;
        *self.login_chrome.lock().await = Some(login);
        Ok(())
    }

    /// Polls interactive Chrome exit state and profile sign-in progress.
    pub(crate) async fn poll_login_chrome(&self) -> Result<Option<LoginPollStatus>> {
        let (exited, profile_dir, started_at) = {
            let mut guard = self.login_chrome.lock().await;
            let Some(lc) = guard.as_mut() else {
                return Ok(None);
            };
            (lc.try_exited()?, lc.profile_dir.clone(), lc.started_at)
        };
        let probe = inspect_login_profile(&profile_dir, started_at).await;
        Ok(Some(LoginPollStatus {
            exited,
            rejected_url: probe.rejected_url,
            has_session_cookie: probe.has_session_cookie,
        }))
    }

    /// Stops interactive Chrome to flush `<home>/chrome-profile` and starts headless CDP Chrome on it.
    pub(crate) async fn promote_login_chrome(&self) -> Result<()> {
        let _ensure = self.ensure_lock.lock().await;
        let stale = self.inner.lock().await.take();
        if let Some(old) = stale {
            shutdown_active(old).await;
        }
        let lc = self
            .login_chrome
            .lock()
            .await
            .take()
            .ok_or_else(|| anyhow::anyhow!("No active interactive Chrome sign-in window"))?;
        lc.shutdown().await;
        self.visible.store(false, Ordering::Release);
        let managed = ManagedChrome::launch(APP_URL).await?;
        let active = self.connect_managed_chrome(managed).await?;
        *self.inner.lock().await = Some(active);
        Ok(())
    }

    /// Connects to a page target on `managed` or shuts down `managed` if connection fails.
    async fn connect_managed_chrome(&self, managed: ManagedChrome) -> Result<ActiveChrome> {
        let ws_url = match find_or_create_page_target(&managed.http_endpoint).await {
            Ok(v) => v,
            Err(err) => {
                managed.shutdown().await;
                return Err(err);
            }
        };
        let cdp = match CdpConnection::connect_page(&ws_url).await {
            Ok(conn) => Arc::new(conn),
            Err(err) => {
                managed.shutdown().await;
                return Err(err);
            }
        };
        if !managed.user_agent.is_empty() {
            *self.user_agent.lock().await = Some(managed.user_agent.clone());
        }
        Ok(ActiveChrome { managed, cdp })
    }

    /// Returns `true` when an active Chrome page CDP connection is open.
    pub(crate) async fn has_page(&self) -> bool {
        let inner = self.inner.lock().await;
        inner.as_ref().is_some_and(|a| a.cdp.is_alive())
    }

    /// Returns `true` when the interactive Chrome sign-in window is active.
    pub(crate) fn is_window_visible(&self) -> bool {
        self.visible.load(Ordering::Acquire)
    }

    /// Returns the local HTTP CDP endpoint URL if Chrome is running.
    pub(crate) async fn cdp_endpoint(&self) -> Option<String> {
        let inner = self.inner.lock().await;
        inner
            .as_ref()
            .filter(|a| a.cdp.is_alive())
            .map(|a| a.managed.http_endpoint.clone())
    }

    /// Ensures a live headless Chrome CDP page exists on the persistent profile.
    pub(crate) async fn ensure_page(&self) -> Result<()> {
        let _ensure = self.ensure_lock.lock().await;
        let stale = {
            let mut inner = self.inner.lock().await;
            match inner.as_ref() {
                Some(a) if a.cdp.is_alive() => return Ok(()),
                Some(_) => inner.take(),
                None => None,
            }
        };
        let login = self.login_chrome.lock().await.take();
        if let Some(old) = stale {
            shutdown_active(old).await;
        }
        if let Some(lc) = login {
            lc.shutdown().await;
        }
        self.visible.store(false, Ordering::Release);
        let managed = ManagedChrome::launch("about:blank").await?;
        let active = self.connect_managed_chrome(managed).await?;
        *self.inner.lock().await = Some(active);
        Ok(())
    }

    /// Returns the active page CDP connection, launching headless Chrome if necessary.
    async fn active_cdp(&self) -> Result<Arc<CdpConnection>> {
        self.ensure_page().await?;
        let inner = self.inner.lock().await;
        inner
            .as_ref()
            .map(|a| Arc::clone(&a.cdp))
            .ok_or_else(|| anyhow::anyhow!("Chrome page target unavailable"))
    }

    /// Navigates the active page to `url` and waits until the document becomes interactive.
    pub(crate) async fn navigate(&self, url: &str) -> Result<()> {
        let cdp = self.active_cdp().await?;
        cdp.call("Page.navigate", json!({ "url": url })).await?;
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let state = eval_runtime(&cdp, "document.readyState", false, Duration::from_secs(5))
                .await
                .unwrap_or(Value::Null);
            if state.as_str() == Some("complete") {
                tokio::time::sleep(Duration::from_millis(500)).await;
                break;
            }
        }
        Ok(())
    }

    /// Returns the current page URL, or an empty string if Chrome has not started.
    pub(crate) async fn current_url(&self) -> Result<String> {
        let cdp = {
            let inner = self.inner.lock().await;
            let Some(a) = inner.as_ref().filter(|a| a.cdp.is_alive()) else {
                return Ok(String::new());
            };
            Arc::clone(&a.cdp)
        };
        let val =
            eval_runtime(&cdp, "window.location.href", false, Duration::from_secs(10)).await?;
        Ok(val.as_str().unwrap_or("").to_string())
    }

    /// Ensures that the active page is on the Google Flow Music origin, which the API calls need.
    pub(crate) async fn ensure_app_page(&self) -> Result<()> {
        self.ensure_page().await?;
        let current = self.current_url().await.unwrap_or_default();
        if !is_app_origin(&current) {
            self.navigate(APP_URL).await?;
        }
        Ok(())
    }

    /// Reloads the application page to let the page client refresh its session token.
    pub(crate) async fn reload_app_page(&self) -> Result<()> {
        self.ensure_page().await?;
        self.navigate(APP_URL).await
    }

    /// Calls an asynchronous method on `window.__flowMusicMcp`, installing the runtime bundle on demand.
    pub(crate) async fn call_runtime(
        &self,
        method: &str,
        args: Value,
        timeout: Duration,
    ) -> Result<Value> {
        anyhow::ensure!(
            args.is_array(),
            "Runtime arguments for '{}' must be a JSON array",
            method
        );
        let method_json = serde_json::to_string(method)?;
        let args_json = serde_json::to_string(&args)?;
        let probe_expr = format!(
            "(async () => {{ const rt = window.__flowMusicMcp; if (!rt || typeof rt[{method_json}] !== \"function\") return {{ __hit: false }}; return {{ __hit: true, value: await rt[{method_json}](...{args_json}) }}; }})()"
        );
        let cdp = self.active_cdp().await?;
        let probe = eval_runtime(&cdp, &probe_expr, true, timeout).await?;
        if probe.get("__hit").and_then(Value::as_bool) == Some(true) {
            return Ok(probe.get("value").cloned().unwrap_or(Value::Null));
        }
        eval_runtime(&cdp, MUSIC_RUNTIME_JS, false, Duration::from_secs(30)).await?;
        let direct_expr =
            format!("(async () => await window.__flowMusicMcp[{method_json}](...{args_json}))()");
        eval_runtime(&cdp, &direct_expr, true, timeout).await
    }

    /// Returns the Chrome `User-Agent` string or the platform Chrome fallback.
    pub(crate) async fn user_agent(&self) -> String {
        self.user_agent
            .lock()
            .await
            .clone()
            .unwrap_or_else(|| DEFAULT_CHROME_UA.to_string())
    }

    /// Stops Chrome processes and deletes `<home>/chrome-profile`.
    pub(crate) async fn clear_session(&self) -> Result<()> {
        let _guard = self.ensure_lock.lock().await;
        let login = self.login_chrome.lock().await.take();
        let active = self.inner.lock().await.take();
        if let Some(lc) = login {
            lc.shutdown().await;
        }
        if let Some(old) = active {
            shutdown_active(old).await;
        }
        self.visible.store(false, Ordering::Release);
        let _ = tokio::fs::remove_dir_all(crate::config::chrome_profile_dir()).await;
        Ok(())
    }
}

/// Returns `true` when `raw` is a page URL on the Google Flow Music application origin.
fn is_app_origin(raw: &str) -> bool {
    let (Ok(page), Ok(app)) = (Url::parse(raw), Url::parse(APP_ORIGIN)) else {
        return false;
    };
    page.scheme() == app.scheme() && page.host_str() == app.host_str()
}

/// Closes the CDP connection and stops the headless Chrome child process.
async fn shutdown_active(a: ActiveChrome) {
    drop(a.cdp);
    a.managed.shutdown().await;
}

/// Evaluates `expr` via CDP `Runtime.evaluate` and propagates JavaScript exception messages.
async fn eval_runtime(
    cdp: &CdpConnection,
    expr: &str,
    await_promise: bool,
    timeout: Duration,
) -> Result<Value> {
    let res = cdp
        .call_with_timeout(
            "Runtime.evaluate",
            json!({
                "expression": expr,
                "returnByValue": true,
                "awaitPromise": await_promise,
            }),
            timeout,
        )
        .await?;
    if let Some(details) = res.get("exceptionDetails") {
        let msg = details
            .pointer("/exception/description")
            .or_else(|| details.pointer("/exception/value"))
            .or_else(|| details.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("JavaScript evaluation exception");
        anyhow::bail!("JavaScript error: {}", msg);
    }
    Ok(res.pointer("/result/value").cloned().unwrap_or(Value::Null))
}
