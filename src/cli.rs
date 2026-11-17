// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Command-line flags, subcommands, and subcommand output for `flow-music-mcp`.

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::auth::{self, LoginParams};
use crate::chrome;
use crate::config::{self, Settings};
use crate::engine::ChromeEngine;
use crate::http::{HttpOptions, serve_http};
use crate::instance;
use crate::mcp::MusicMcpServer;
use crate::platform;
use crate::service::MusicService;

/// Largest pause bound in seconds that `--pause` accepts.
const MAX_PAUSE_SECS: f64 = 86_400.0;

/// Time limit for `<chrome> --version` during `doctor`.
const CHROME_VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// Command-line flags and the subcommand for `flow-music-mcp`.
#[derive(Debug, Parser)]
#[command(
    name = "flow-music-mcp",
    version,
    about = "Rust MCP server for Google Flow Music with a Chrome CDP browser engine",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub(crate) struct Cli {
    /// Set the data root directory (defaults to ~/.flow-music-mcp)
    #[arg(long, value_name = "PATH", env = "FLOW_MUSIC_MCP_HOME", global = true)]
    home: Option<PathBuf>,

    /// Set the Chrome or Chromium binary path (defaults to auto-detect)
    #[arg(
        long,
        value_name = "PATH",
        env = "FLOW_MUSIC_MCP_CHROME",
        global = true
    )]
    chrome: Option<PathBuf>,

    /// Set a fixed Chrome DevTools Protocol port (defaults to a free port)
    #[arg(
        long,
        value_name = "PORT",
        env = "FLOW_MUSIC_MCP_CDP_PORT",
        global = true,
        value_parser = clap::value_parser!(u16).range(1..)
    )]
    cdp_port: Option<u16>,

    /// Set the random pause between queue jobs in seconds (one value sets a fixed pause, 0 disables it)
    #[arg(
        long,
        value_name = "MIN[,MAX]",
        env = "FLOW_MUSIC_MCP_PAUSE",
        global = true,
        default_value = "5,15",
        value_parser = parse_pause
    )]
    pause: (u64, u64),

    /// Subcommand to run.
    #[command(subcommand)]
    command: Command,
}

/// Subcommands of `flow-music-mcp`.
#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the MCP tools over standard input and output, or over Streamable HTTP with --http
    Serve {
        /// Serve MCP over Streamable HTTP at http://ADDR/mcp instead of standard input and output (for example 127.0.0.1:8765)
        #[arg(long, value_name = "IP:PORT", env = "FLOW_MUSIC_MCP_HTTP")]
        http: Option<SocketAddr>,
        /// Require this bearer token on each HTTP request; required when the address is not a loopback address
        #[arg(
            long,
            value_name = "TOKEN",
            env = "FLOW_MUSIC_MCP_HTTP_TOKEN",
            hide_env_values = true,
            value_parser = clap::builder::NonEmptyStringValueParser::new()
        )]
        http_token: Option<String>,
    },
    /// Sign in to Google Flow Music in a visible Chrome window and wait until sign-in completes
    Login {
        /// Set the maximum wait time for sign-in in seconds (15 to 3600)
        #[arg(long, value_name = "SECS", default_value_t = 180)]
        timeout: u64,
        /// Delete the saved Chrome profile before sign-in starts
        #[arg(long)]
        force: bool,
        /// Write the result as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show the Google Flow Music sign-in status (starts headless Chrome)
    Status {
        /// Write the result as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete the saved Chrome profile and the Google Flow Music session
    Logout {
        /// Write the result as JSON
        #[arg(long)]
        json: bool,
    },
    /// Check the Chrome binary, the data directory, the instance lock, and the sign-in status
    Doctor {
        /// Write the result as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Parses one pause bound in seconds into milliseconds.
fn parse_pause_secs(raw: &str) -> Result<u64, String> {
    let secs = raw
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("`{}` is not a number of seconds", raw.trim()))?;
    if !secs.is_finite() || !(0.0..=MAX_PAUSE_SECS).contains(&secs) {
        return Err(format!(
            "`{}` is not between 0 and {} seconds",
            raw.trim(),
            MAX_PAUSE_SECS
        ));
    }
    Ok((secs * 1000.0).round() as u64)
}

/// Parses `MIN[,MAX]` in seconds into inclusive pause bounds in milliseconds.
fn parse_pause(raw: &str) -> Result<(u64, u64), String> {
    let (min_ms, max_ms) = match raw.split_once(',') {
        Some((min, max)) => (parse_pause_secs(min)?, parse_pause_secs(max)?),
        None => {
            let fixed = parse_pause_secs(raw)?;
            (fixed, fixed)
        }
    };
    if min_ms > max_ms {
        return Err("MIN must not be greater than MAX".to_string());
    }
    Ok((min_ms, max_ms))
}

/// Waits for `SIGINT` or Unix `SIGTERM`.
async fn wait_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// Runs `task` until it ends or a shutdown signal arrives, then shuts down Chrome.
async fn run_with_engine<T>(
    engine: &Arc<ChromeEngine>,
    task: impl Future<Output = Result<T>>,
) -> Result<T> {
    let result = tokio::select! {
        res = task => res,
        _ = wait_shutdown_signal() => Err(anyhow::anyhow!("Interrupted by a shutdown signal")),
    };
    engine.shutdown().await;
    result
}

/// Returns `LoginParams` for `action` with default values for all other fields.
fn login_params(action: &str) -> LoginParams {
    LoginParams {
        action: Some(action.to_string()),
        wait: None,
        timeout_secs: None,
        force: None,
    }
}

/// Takes the `<home>` lock, runs one `music_login` action in a new `ChromeEngine`, and returns the tool result.
async fn run_login_action(params: LoginParams) -> Result<Value> {
    instance::acquire_instance_lock()?;
    let engine = ChromeEngine::new();
    run_with_engine(&engine, auth::handle_login(&engine, params)).await
}

/// Formats one JSON value as plain text for terminal output.
fn plain_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "yes".to_string(),
        Value::Bool(false) => "no".to_string(),
        Value::Null => "-".to_string(),
        other => other.to_string(),
    }
}

/// Writes `result` as pretty JSON or as one `key: value` line per top-level field.
fn print_result(result: &Value, as_json: bool) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(result)?);
        return Ok(());
    }
    if let Value::Object(map) = result {
        for (key, value) in map {
            if key != "ok" {
                println!("{key}: {}", plain_value(value));
            }
        }
    } else {
        println!("{}", plain_value(result));
    }
    Ok(())
}

/// Returns one `doctor` check result.
fn check(name: &str, status: &str, detail: impl Into<String>) -> Value {
    json!({ "name": name, "status": status, "detail": detail.into() })
}

/// Runs `<chrome> --version` and returns its first output line.
async fn chrome_version(path: &Path) -> Option<String> {
    let output = tokio::time::timeout(
        CHROME_VERSION_TIMEOUT,
        tokio::process::Command::new(path)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

/// Checks that the Chrome binary exists and reports its version.
async fn check_chrome() -> Value {
    let source = if config::cli_chrome_path().is_some() {
        "--chrome"
    } else {
        "auto-detect"
    };
    let Some(path) = chrome::find_chrome() else {
        return check(
            "chrome",
            "fail",
            "No Chrome or Chromium found. Install Chrome, or set --chrome or FLOW_MUSIC_MCP_CHROME.",
        );
    };
    if !path.is_file() {
        return check(
            "chrome",
            "fail",
            format!("{} ({source}) is not a file", path.display()),
        );
    }
    let version = chrome_version(&path)
        .await
        .unwrap_or_else(|| "version unknown".to_string());
    check(
        "chrome",
        "ok",
        format!("{} ({source}, {version})", path.display()),
    )
}

/// Checks that the Unix permission bits of `path` are `expected`.
fn check_mode(name: &str, path: &Path, expected: u32) -> Value {
    if !path.exists() {
        return check(
            name,
            "skip",
            format!("{} does not exist yet", path.display()),
        );
    }
    match platform::file_mode(path) {
        None => check(
            name,
            "skip",
            "Unix permission bits do not apply on this platform",
        ),
        Some(mode) => {
            let status = if mode == expected { "ok" } else { "fail" };
            check(
                name,
                status,
                format!(
                    "{} has mode {mode:o}, expected {expected:o}",
                    path.display()
                ),
            )
        }
    }
}

/// Checks the data root and the permissions of the private files in it.
fn check_home() -> Vec<Value> {
    let home = config::home_dir();
    let home_check = match std::fs::metadata(&home) {
        Err(_) => check(
            "home",
            "warn",
            format!("{} does not exist yet", home.display()),
        ),
        Ok(meta) if !meta.is_dir() => check(
            "home",
            "fail",
            format!("{} is not a directory", home.display()),
        ),
        Ok(meta) if meta.permissions().readonly() => {
            check("home", "fail", format!("{} is read-only", home.display()))
        }
        Ok(_) => check("home", "ok", home.display().to_string()),
    };
    vec![
        home_check,
        check_mode("chrome-profile", &config::chrome_profile_dir(), 0o700),
        check_mode("flow-music-mcp.db", &config::db_path(), 0o600),
    ]
}

/// Takes the `<home>` lock and reports orphaned Chrome; returns the check and `true` when this process holds the lock.
fn check_instance_lock() -> (Value, bool) {
    match instance::try_acquire_instance_lock() {
        Err(e) => (check("instance-lock", "fail", format!("{e:#}")), false),
        Ok(false) => (
            check(
                "instance-lock",
                "warn",
                format!(
                    "Another flow-music-mcp process uses {}",
                    config::home_dir().display()
                ),
            ),
            false,
        ),
        Ok(true) => {
            let orphans = instance::orphan_chrome_pids(&config::chrome_profile_dir());
            let value = if orphans.is_empty() {
                check(
                    "instance-lock",
                    "ok",
                    "No other flow-music-mcp process uses the data directory",
                )
            } else {
                check(
                    "instance-lock",
                    "warn",
                    format!(
                        "Orphaned Chrome PID {orphans:?} uses the profile; the next Chrome start stops it"
                    ),
                )
            };
            (value, true)
        }
    }
}

/// Checks the Google Flow Music sign-in status in headless Chrome when this process holds the `<home>` lock.
async fn check_sign_in(locked: bool) -> Value {
    if !locked {
        return check(
            "sign-in",
            "skip",
            "Another flow-music-mcp uses the data directory; use the music_login tool there",
        );
    }
    match run_login_action(login_params("status")).await {
        Err(e) => check("sign-in", "fail", format!("{e:#}")),
        Ok(res) if res.get("signedIn").and_then(Value::as_bool) == Some(true) => {
            let url = res.get("url").map(plain_value).unwrap_or_default();
            check("sign-in", "ok", format!("Signed in ({url})"))
        }
        Ok(_) => check(
            "sign-in",
            "warn",
            "Not signed in; run `flow-music-mcp login`",
        ),
    }
}

/// Runs all `doctor` checks and writes the report.
async fn run_doctor(as_json: bool) -> Result<()> {
    let mut checks = vec![check_chrome().await];
    checks.extend(check_home());
    let (lock_check, locked) = check_instance_lock();
    checks.push(lock_check);
    checks.push(check_sign_in(locked).await);

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "checks": checks }))?
        );
        return Ok(());
    }
    for c in &checks {
        let status = c.get("status").map(plain_value).unwrap_or_default();
        let name = c.get("name").map(plain_value).unwrap_or_default();
        let detail = c.get("detail").map(plain_value).unwrap_or_default();
        println!("[{status:<4}] {name}: {detail}");
    }
    Ok(())
}

/// Starts the MCP server over standard input and output, or over HTTP when `http` is set,
/// and shuts down Chrome on exit or on a shutdown signal.
async fn run_serve(http: Option<HttpOptions>) -> Result<()> {
    let engine = ChromeEngine::new();
    let server = MusicMcpServer::new(MusicService::new(Arc::clone(&engine)));
    let result = match http {
        None => tokio::select! {
            res = server.serve_stdio() => res,
            _ = wait_shutdown_signal() => Ok(()),
        },
        Some(options) => serve_http(server, options, wait_shutdown_signal()).await,
    };
    engine.shutdown().await;
    result
}

/// Stores the parsed settings and runs the selected subcommand.
pub(crate) async fn run(cli: Cli) -> Result<()> {
    config::init_settings(Settings {
        home: cli.home,
        chrome: cli.chrome,
        cdp_port: cli.cdp_port,
        pause_ms: cli.pause,
    });
    match cli.command {
        Command::Serve { http, http_token } => {
            run_serve(http.map(|addr| HttpOptions {
                addr,
                token: http_token,
            }))
            .await
        }
        Command::Login {
            timeout,
            force,
            json,
        } => {
            let params = LoginParams {
                wait: Some(true),
                timeout_secs: Some(timeout),
                force: Some(force),
                ..login_params("login")
            };
            eprintln!(
                "Complete the sign-in in the browser window. flow-music-mcp continues when it detects the sign-in, or when you close the window."
            );
            print_result(&run_login_action(params).await?, json)
        }
        Command::Status { json } => {
            print_result(&run_login_action(login_params("status")).await?, json)
        }
        Command::Logout { json } => {
            print_result(&run_login_action(login_params("logout")).await?, json)
        }
        Command::Doctor { json } => run_doctor(json).await,
    }
}
