// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! CLI entry point for repository checks and CI gates.

mod ci;
mod examples;
mod forbidden;
mod headers;
mod json;
mod lexer;
mod licenses;
mod lines;
mod npm;
mod workspace;

use std::process::ExitCode;

/// Dispatches `cargo xtask <subcommand>` commands.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");

    if matches!(cmd, "help" | "--help" | "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }

    let root = match workspace::workspace_root() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let sub_args = args.get(1..).unwrap_or(&[]);
    let result = match cmd {
        "ci" => ci::run_ci(&root),
        "check" => ci::run_all_checks(&root),
        "license-headers" => ci::run_license_headers(&root),
        "line-limit" => ci::run_line_limit(&root),
        "forbidden-code" => ci::run_forbidden_code(&root),
        "collect-licenses" => licenses::run_collect_licenses(&root),
        "npm-pack" => npm::run_npm_pack(&root, sub_args),
        other => Err(format!("Unknown xtask command: {other}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Prints available `xtask` subcommands to standard output.
fn print_help() {
    println!(
        "flow-music-mcp xtask\n\nCommands:\n  ci                              Run fmt, repository checks, clippy, doc, build, and cargo deny\n  check                           Run license-headers, line-limit, doc examples, and forbidden-code checks\n  license-headers                 Verify SPDX headers, shebang launcher headers, and LF endings\n  line-limit                      Verify <= 800 code lines per .rs and .ts file\n  forbidden-code                  Verify zero-tests, layout, manifest, version sync, and TypeScript rules\n  collect-licenses                Collect license notices for locked Rust crate dependencies\n  npm-pack --dist <D> --out <O>   Build @flow-music-mcp/cli and six platform packages from release binaries"
    );
}
