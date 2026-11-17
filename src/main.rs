// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Binary entry point for `flow-music-mcp`.

mod auth;
mod cdp;
mod chat;
mod chrome;
mod cli;
mod client;
mod collections;
mod config;
mod db;
mod download;
mod edits;
mod engine;
mod generation;
mod http;
mod instance;
mod library;
mod mcp;
mod media_files;
mod platform;
mod producer;
mod queue;
mod service;
mod uploads;

use anyhow::Result;
use clap::Parser;
use std::time::Duration;

/// Time that blocking tasks, such as SQLite writes, get to finish after the subcommand ends.
///
/// Tokio reads standard input with a blocking read that it cannot cancel. Without this limit,
/// `serve` in a terminal does not stop after `Ctrl-C` until standard input receives a line.
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);

/// Parses the command line, runs the selected subcommand, and stops the runtime within a time limit.
fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(cli::run(cli));
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
    result
}
