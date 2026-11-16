// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Binary entry point for `flow-music-mcp`.

mod auth;
mod cdp;
mod chat;
mod chrome;
mod client;
mod collections;
mod config;
mod db;
mod download;
mod edits;
mod engine;
mod generation;
mod library;
mod mcp;
mod media_files;
mod private_fs;
mod producer;
mod queue;
mod service;
mod uploads;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;

use engine::ChromeEngine;
use mcp::MusicMcpServer;
use service::MusicService;

/// Command-line flags for `flow-music-mcp`.
#[derive(Debug, Parser)]
#[command(
    name = "flow-music-mcp",
    version,
    about = "Rust MCP server for Google Flow Music with a Chrome CDP browser engine"
)]
struct Cli {
    /// Set the data root directory (defaults to FLOW_MUSIC_MCP_HOME or ~/.flow-music-mcp)
    #[arg(long, value_name = "PATH")]
    home: Option<PathBuf>,

    /// Set the Chrome or Chromium binary path (defaults to FLOW_MUSIC_MCP_CHROME or auto-detect)
    #[arg(long, value_name = "PATH")]
    chrome: Option<PathBuf>,
}

/// Waits for `SIGINT` or Unix `SIGTERM` to initiate graceful Chrome shutdown.
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

/// Parses CLI flags, starts the stdio MCP server, and shuts down Chrome on exit.
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    config::init_cli_home(cli.home);
    if let Some(chrome) = cli.chrome {
        config::set_chrome_path(chrome);
    }

    let engine = ChromeEngine::new();
    let server = MusicMcpServer::new(MusicService::new(Arc::clone(&engine)));
    let result = tokio::select! {
        res = server.serve_stdio() => res,
        _ = wait_shutdown_signal() => Ok(()),
    };
    engine.shutdown().await;
    result
}
