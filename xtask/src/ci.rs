// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Ordered repository checks and CI command execution.

use std::path::Path;
use std::process::Command;

use crate::examples::check_doc_examples;
use crate::forbidden::check_forbidden_code;
use crate::headers::check_license_headers;
use crate::lines::check_line_limits;
use crate::workspace::collect_files;

/// Runs license header, code line limit, doc example, and forbidden code checks.
pub(crate) fn run_all_checks(root: &Path) -> Result<(), String> {
    let files = collect_files(root)?;
    let mut errors = Vec::new();
    errors.extend(check_license_headers(root, &files));
    errors.extend(check_line_limits(root, &files));
    errors.extend(check_doc_examples(root, &files));
    errors.extend(check_forbidden_code(root, &files));

    if errors.is_empty() {
        return Ok(());
    }
    for err in &errors {
        eprintln!("CHECK ERROR: {err}");
    }
    Err(format!("{} repository check error(s) found", errors.len()))
}

/// Runs license header and LF line ending checks only.
pub(crate) fn run_license_headers(root: &Path) -> Result<(), String> {
    let files = collect_files(root)?;
    report_errors(check_license_headers(root, &files))
}

/// Runs code line limit checks only.
pub(crate) fn run_line_limit(root: &Path) -> Result<(), String> {
    let files = collect_files(root)?;
    report_errors(check_line_limits(root, &files))
}

/// Runs forbidden code, doc example, manifest, and TypeScript type checks only.
pub(crate) fn run_forbidden_code(root: &Path) -> Result<(), String> {
    let files = collect_files(root)?;
    let mut errors = check_doc_examples(root, &files);
    errors.extend(check_forbidden_code(root, &files));
    report_errors(errors)
}

/// Executes the full CI gate and prints `CI: OK` on success.
pub(crate) fn run_ci(root: &Path) -> Result<(), String> {
    run_cargo(root, &["fmt", "--all", "--", "--check"], &[])?;
    run_all_checks(root)?;
    run_cargo(
        root,
        &[
            "clippy",
            "--locked",
            "--offline",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        &[],
    )?;
    run_cargo(
        root,
        &[
            "doc",
            "--locked",
            "--offline",
            "--workspace",
            "--no-deps",
            "--document-private-items",
        ],
        &[("RUSTDOCFLAGS", "-D warnings")],
    )?;
    run_cargo(
        root,
        &["build", "--locked", "--offline", "--workspace"],
        &[],
    )?;
    run_cargo(
        root,
        &["deny", "--locked", "--offline", "--workspace", "check"],
        &[],
    )?;
    println!("CI: OK");
    Ok(())
}

/// Prints `errors` and fails when `errors` is non-empty.
fn report_errors(errors: Vec<String>) -> Result<(), String> {
    if errors.is_empty() {
        return Ok(());
    }
    for err in &errors {
        eprintln!("CHECK ERROR: {err}");
    }
    Err(format!("{} check error(s) found", errors.len()))
}

/// Runs a Cargo subcommand inside `root` with optional environment overrides.
fn run_cargo(root: &Path, args: &[&str], envs: &[(&str, &str)]) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let status = cmd
        .status()
        .map_err(|err| format!("Failed to spawn `cargo {}`: {err}", args.join(" ")))?;
    if !status.success() {
        return Err(format!(
            "`cargo {}` failed with status {status}",
            args.join(" ")
        ));
    }
    Ok(())
}
