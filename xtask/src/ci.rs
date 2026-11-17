// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Ordered repository checks and CI command execution.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::examples::check_doc_examples;
use crate::forbidden::check_forbidden_code;
use crate::headers::check_license_headers;
use crate::lines::check_line_limits;
use crate::workspace::collect_files;

/// Target triple for the optional local Windows Clippy pass.
const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";

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
    run_windows_clippy(root)?;
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

/// Returns why the local Windows Clippy pass cannot run, or `None` when it can run.
fn windows_clippy_unavailable(root: &Path) -> Option<String> {
    let has_xwin = Command::new("cargo")
        .current_dir(root)
        .args(["xwin", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !has_xwin {
        return Some("install cargo-xwin to check Windows code on this computer".to_string());
    }
    let libdir = Command::new("rustc")
        .current_dir(root)
        .args(["--print", "target-libdir", "--target", WINDOWS_TARGET])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string());
    match libdir {
        Some(dir) if Path::new(&dir).is_dir() => None,
        _ => Some(format!(
            "the rustc in PATH has no {WINDOWS_TARGET} standard library; run `rustup target add {WINDOWS_TARGET}` and put the rustup toolchain first in PATH"
        )),
    }
}

/// Runs Clippy for Windows with `cargo xwin`, or prints why this optional pass is skipped.
fn run_windows_clippy(root: &Path) -> Result<(), String> {
    if let Some(reason) = windows_clippy_unavailable(root) {
        println!("SKIP: Windows Clippy: {reason}");
        return Ok(());
    }
    run_cargo(
        root,
        &[
            "xwin",
            "clippy",
            "--locked",
            "--offline",
            "--workspace",
            "--all-targets",
            "--target",
            WINDOWS_TARGET,
            "--",
            "-D",
            "warnings",
        ],
        &[],
    )
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
