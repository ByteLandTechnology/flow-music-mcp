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

/// Parses the command line and runs the selected subcommand.
#[tokio::main]
async fn main() -> Result<()> {
    cli::run(cli::Cli::parse()).await
}
