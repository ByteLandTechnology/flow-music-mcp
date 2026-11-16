// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Workspace root resolution and deterministic source file discovery.

use std::fs;
use std::path::{Path, PathBuf};

/// Directories skipped during recursive workspace traversal.
const SKIP_DIRS: &[&str] = &[
    "target",
    "artifacts",
    "node_modules",
    "vendor",
    "out",
    "dist",
    "npm-dist",
];

/// Directory names reserved for tests and forbidden in this repository.
const TEST_DIRS: &[&str] = &["test", "tests", "benches", "__tests__"];

/// Categorized file inventory collected across the workspace.
#[derive(Debug, Default)]
pub(crate) struct WorkspaceFiles {
    /// All Rust source files under the workspace.
    pub(crate) rust_files: Vec<PathBuf>,
    /// All TypeScript source files directly under `ts/`.
    pub(crate) ts_files: Vec<PathBuf>,
    /// Allowed JavaScript launcher scripts under `npm/`.
    pub(crate) launcher_js_files: Vec<PathBuf>,
    /// Stray JavaScript, declaration, or misplaced TypeScript script files.
    pub(crate) stray_script_files: Vec<PathBuf>,
    /// All `Cargo.toml` manifests in the workspace.
    pub(crate) cargo_manifests: Vec<PathBuf>,
    /// All Markdown documents in the workspace.
    pub(crate) markdown_files: Vec<PathBuf>,
    /// All source and text files checked for LF line endings.
    pub(crate) text_files: Vec<PathBuf>,
    /// Discovered forbidden test or benchmark directories.
    pub(crate) test_dirs: Vec<PathBuf>,
    /// Subdirectories inside flat source roots (`src/`, `xtask/src/`, `ts/`).
    pub(crate) nested_source_dirs: Vec<PathBuf>,
}

/// Resolves the workspace root directory from `CARGO_MANIFEST_DIR`.
pub(crate) fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Failed to resolve workspace root from CARGO_MANIFEST_DIR".to_string())
}

/// Formats `path` relative to `root` with forward slashes.
pub(crate) fn rel_display(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

/// Collects all checked workspace files and layout violations in sorted order.
pub(crate) fn collect_files(root: &Path) -> Result<WorkspaceFiles, String> {
    let mut out = WorkspaceFiles::default();
    walk_dir(root, root, &mut out)?;
    out.rust_files.sort();
    out.ts_files.sort();
    out.launcher_js_files.sort();
    out.stray_script_files.sort();
    out.cargo_manifests.sort();
    out.markdown_files.sort();
    out.text_files.sort();
    out.test_dirs.sort();
    out.nested_source_dirs.sort();
    Ok(out)
}

/// Recursively walks `dir` while skipping dot-directories and build outputs.
fn walk_dir(root: &Path, dir: &Path, out: &mut WorkspaceFiles) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|err| format!("Failed to read directory {}: {err}", rel_display(root, dir)))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            format!(
                "Failed to read directory entry in {}: {err}",
                rel_display(root, dir)
            )
        })?;
        paths.push(entry.path());
    }
    paths.sort();

    for path in paths {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let meta = fs::symlink_metadata(&path)
            .map_err(|err| format!("Failed to stat {}: {err}", rel_display(root, &path)))?;
        if meta.is_dir() {
            if name.starts_with('.') || SKIP_DIRS.contains(&name) {
                continue;
            }
            if TEST_DIRS.contains(&name) {
                out.test_dirs.push(path);
                continue;
            }
            if is_inside_flat_root(root, &path) {
                out.nested_source_dirs.push(path.clone());
            }
            walk_dir(root, &path, out)?;
        } else if meta.is_file() {
            record_file(root, &path, name, out);
        }
    }
    Ok(())
}

/// Returns `true` when `dir` is a subdirectory of `src/`, `xtask/src/`, or `ts/`.
fn is_inside_flat_root(root: &Path, dir: &Path) -> bool {
    let Ok(rel) = dir.strip_prefix(root) else {
        return false;
    };
    let s = rel.to_string_lossy().replace('\\', "/");
    (s.starts_with("src/") && s != "src")
        || (s.starts_with("xtask/src/") && s != "xtask/src")
        || (s.starts_with("ts/") && s != "ts")
}

/// Classifies one regular file into `out` lists by extension and file name.
fn record_file(root: &Path, path: &Path, name: &str, out: &mut WorkspaceFiles) {
    if name == "Cargo.toml" {
        out.cargo_manifests.push(path.to_path_buf());
        out.text_files.push(path.to_path_buf());
        return;
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => {
            out.rust_files.push(path.to_path_buf());
            out.text_files.push(path.to_path_buf());
        }
        Some("ts") => {
            let rel = rel_display(root, path);
            if rel.starts_with("ts/") && !name.ends_with(".d.ts") {
                out.ts_files.push(path.to_path_buf());
            } else {
                out.stray_script_files.push(path.to_path_buf());
            }
            out.text_files.push(path.to_path_buf());
        }
        Some("js" | "mjs" | "cjs" | "jsx" | "tsx") => {
            let rel = rel_display(root, path);
            if rel == "npm/cli/bin/flow-music-mcp.js" {
                out.launcher_js_files.push(path.to_path_buf());
            } else {
                out.stray_script_files.push(path.to_path_buf());
            }
            out.text_files.push(path.to_path_buf());
        }
        Some("md") => {
            out.markdown_files.push(path.to_path_buf());
            out.text_files.push(path.to_path_buf());
        }
        Some("toml" | "json" | "svg") => {
            out.text_files.push(path.to_path_buf());
        }
        _ => {}
    }
}

/// Reads the `version` value from the `[package]` table of a `Cargo.toml` manifest.
pub(crate) fn cargo_package_version(manifest: &str) -> Result<String, String> {
    let mut in_package = false;
    for raw_line in manifest.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = line.strip_prefix("version") {
            let trimmed_rest = rest.trim_start();
            if let Some(after_eq) = trimmed_rest.strip_prefix('=') {
                let value = after_eq
                    .split('#')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"');
                if !value.is_empty() {
                    return Ok(value.to_owned());
                }
            }
        }
    }
    Err("Cargo.toml has no [package] version".to_owned())
}
