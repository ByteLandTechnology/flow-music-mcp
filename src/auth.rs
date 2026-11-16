// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Google Sign-In for Google Flow Music, session status, and logout handling.
//!
//! Runs Chrome in headless mode during background operations and opens a
//! visible Chrome window only during interactive Google Sign-In. The session
//! lives only in the persistent Chrome profile at `<home>/chrome-profile`.

use anyhow::Result;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::Instant;

use crate::config::{LOGIN_URL, chrome_profile_dir, has_persisted_session};
use crate::engine::{ChromeEngine, ENGINE_NAME};

/// Host that stores the Google Flow Music session cookie.
const SESSION_COOKIE_HOST: &str = "www.flowmusic.app";

/// Name prefix of the Supabase session cookie and its numbered chunks.
const SESSION_COOKIE_PREFIX: &str = "sb-sb-auth-token";

/// Parameters for `music_login`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct LoginParams {
    /// Action name: "login" (alias "start"), "status", or "logout" (alias "reset_session").
    pub(crate) action: Option<String>,
    /// Wait during "login" until sign-in finishes in Chrome (default true).
    pub(crate) wait: Option<bool>,
    /// Maximum wait time in seconds for sign-in completion (default 180, range 15 to 3600).
    pub(crate) timeout_secs: Option<u64>,
    /// Delete the saved Chrome profile before interactive sign-in starts (default false).
    pub(crate) force: Option<bool>,
}

/// Returns `true` when the cookie `name` on `host` is the Google Flow Music session cookie.
pub(crate) fn is_music_session_cookie(host: &str, name: &str) -> bool {
    let host = host.trim_start_matches('.').to_ascii_lowercase();
    let host_ok = host == SESSION_COOKIE_HOST || host == "flowmusic.app";
    let name_ok = name == SESSION_COOKIE_PREFIX
        || name
            .strip_prefix(SESSION_COOKIE_PREFIX)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()));
    host_ok && name_ok
}

/// Reads the sign-in state from the runtime and reloads the page once when the token expired.
pub(crate) async fn auth_state(engine: &ChromeEngine) -> Result<Value> {
    engine.ensure_app_page().await?;
    let state = engine
        .call_runtime("checkAuth", json!([]), Duration::from_secs(30))
        .await?;
    if state.get("token_expired").and_then(Value::as_bool) == Some(true) {
        engine.reload_app_page().await?;
        return engine
            .call_runtime("checkAuth", json!([]), Duration::from_secs(30))
            .await;
    }
    Ok(state)
}

/// Returns `true` when the active page has a confirmed Google Flow Music session.
async fn is_signed_in(engine: &ChromeEngine) -> bool {
    match auth_state(engine).await {
        Ok(state) => state.get("signed_in").and_then(Value::as_bool) == Some(true),
        Err(_) => false,
    }
}

/// Formats a successful `music_login` response with account details from `state`.
async fn signed_in_result(engine: &ChromeEngine, mode: &str) -> Value {
    let state = auth_state(engine).await.unwrap_or(Value::Null);
    json!({
        "ok": true,
        "action": "login",
        "signedIn": true,
        "engine": ENGINE_NAME,
        "mode": mode,
        "userId": state.get("user_id").cloned().unwrap_or(Value::Null),
        "email": state.get("email").cloned().unwrap_or(Value::Null),
        "url": engine.current_url().await.unwrap_or_default(),
    })
}

/// Promotes the interactive Chrome profile into headless CDP mode and verifies the session.
async fn finalize_promoted_login(engine: &ChromeEngine) -> Result<Option<Value>> {
    engine.promote_login_chrome().await?;
    for _ in 0..20 {
        if is_signed_in(engine).await {
            return Ok(Some(signed_in_result(engine, "interactive-chrome").await));
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
    }
    engine.shutdown().await;
    Ok(None)
}

/// Returns the error for a sign-in that Google Accounts rejected, after the session is cleared.
async fn rejected_error(engine: &ChromeEngine, url: &str) -> anyhow::Error {
    let _ = engine.clear_session().await;
    anyhow::anyhow!(
        "Google Accounts rejected sign-in in this browser session (URL: {}).",
        url
    )
}

/// Signs in with standard Chrome and promotes the session into headless CDP Chrome.
async fn login_with_chrome(
    engine: &ChromeEngine,
    timeout_secs: u64,
    wait_for_login: bool,
    force: bool,
) -> Result<Value> {
    if force {
        engine.clear_session().await?;
        engine.launch_interactive_login(LOGIN_URL).await?;
    } else if engine.has_login_chrome().await {
        if let Some(st) = engine.poll_login_chrome().await? {
            if let Some(rejected) = st.rejected_url {
                return Err(rejected_error(engine, &rejected).await);
            }
            if (st.exited || st.has_session_cookie)
                && let Some(res) = finalize_promoted_login(engine).await?
            {
                return Ok(res);
            }
            if st.exited {
                anyhow::bail!("Sign-in window was closed before sign-in completed.");
            }
        }
    } else {
        if (has_persisted_session() || engine.has_page().await) && is_signed_in(engine).await {
            return Ok(signed_in_result(engine, "existing-session").await);
        }
        engine.launch_interactive_login(LOGIN_URL).await?;
    }

    if !wait_for_login {
        return Ok(json!({
            "ok": true,
            "action": "login",
            "signedIn": false,
            "engine": ENGINE_NAME,
            "mode": "window-open",
            "windowVisible": true,
            "url": LOGIN_URL,
            "message": "A Chrome window is open. Select Log in, finish Google Sign-In and the age check, then call music_login again."
        }));
    }

    let clamped_secs = timeout_secs.clamp(15, 3600);
    let deadline = Instant::now() + Duration::from_secs(clamped_secs);
    loop {
        let Some(st) = engine.poll_login_chrome().await? else {
            anyhow::bail!("Sign-in window was closed before sign-in completed.");
        };
        if let Some(rejected) = st.rejected_url {
            return Err(rejected_error(engine, &rejected).await);
        }
        if st.exited || st.has_session_cookie {
            if st.has_session_cookie && !st.exited {
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
            if let Some(res) = finalize_promoted_login(engine).await? {
                return Ok(res);
            }
            anyhow::bail!(
                "The sign-in window closed, but the Google Flow Music session was not confirmed. Run music_login again."
            );
        }
        if Instant::now() >= deadline {
            if let Ok(Some(res)) = finalize_promoted_login(engine).await {
                return Ok(res);
            }
            engine.shutdown().await;
            anyhow::bail!(
                "Timed out after {}s waiting for sign-in in Chrome.",
                clamped_secs
            );
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Reports the session state without opening a sign-in window.
async fn login_status(engine: &ChromeEngine) -> Result<Value> {
    if engine.has_login_chrome().await
        && let Some(st) = engine.poll_login_chrome().await?
    {
        if st.exited || st.has_session_cookie {
            let _ = finalize_promoted_login(engine).await;
        } else {
            return Ok(json!({
                "ok": true,
                "action": "status",
                "engine": ENGINE_NAME,
                "pageReady": false,
                "windowVisible": true,
                "profileDir": chrome_profile_dir().to_string_lossy(),
                "signedIn": false,
            }));
        }
    }
    let mut page_ready = engine.has_page().await;
    if !page_ready && has_persisted_session() && engine.ensure_page().await.is_ok() {
        page_ready = engine.has_page().await;
    }
    let state = if page_ready {
        auth_state(engine).await.unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    Ok(json!({
        "ok": true,
        "action": "status",
        "engine": ENGINE_NAME,
        "pageReady": page_ready,
        "windowVisible": engine.is_window_visible(),
        "cdpEndpoint": engine.cdp_endpoint().await,
        "profileDir": chrome_profile_dir().to_string_lossy(),
        "url": engine.current_url().await.unwrap_or_default(),
        "signedIn": state.get("signed_in").and_then(Value::as_bool).unwrap_or(false),
        "userId": state.get("user_id").cloned().unwrap_or(Value::Null),
        "email": state.get("email").cloned().unwrap_or(Value::Null),
    }))
}

/// Dispatches `music_login` actions (`login`, `status`, and `logout`).
pub(crate) async fn handle_login(engine: &ChromeEngine, req: LoginParams) -> Result<Value> {
    match req.action.as_deref().unwrap_or("login") {
        "login" | "start" => {
            login_with_chrome(
                engine,
                req.timeout_secs.unwrap_or(180),
                req.wait.unwrap_or(true),
                req.force.unwrap_or(false),
            )
            .await
        }
        "status" => login_status(engine).await,
        "logout" | "reset_session" => {
            engine.clear_session().await?;
            Ok(json!({
                "ok": true,
                "action": "logout",
                "engine": ENGINE_NAME,
                "profileDeleted": true,
            }))
        }
        other => anyhow::bail!("Unknown login action: {}", other),
    }
}
