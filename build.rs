// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Cargo build script that bundles `ts/index.ts` into `$OUT_DIR/music_runtime.js`.

use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolves the workspace root and compiles the browser TypeScript bundle via local `esbuild`.
fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .ok_or("missing CARGO_MANIFEST_DIR environment variable")?,
    );
    let out_dir =
        PathBuf::from(env::var_os("OUT_DIR").ok_or("missing OUT_DIR environment variable")?);

    emit_rerun_directives(&manifest_dir)?;

    let esbuild_bin = if cfg!(windows) {
        "esbuild.cmd"
    } else {
        "esbuild"
    };
    let esbuild_path = manifest_dir
        .join("node_modules")
        .join(".bin")
        .join(esbuild_bin);
    if !esbuild_path.is_file() {
        return Err(format!(
            "missing {}; run `npm ci` before building",
            esbuild_path.display()
        )
        .into());
    }

    let out_bundle = out_dir.join("music_runtime.js");
    let output = Command::new(&esbuild_path)
        .current_dir(&manifest_dir)
        .arg("ts/index.ts")
        .arg("--bundle")
        .arg("--format=iife")
        .arg("--platform=browser")
        .arg("--target=es2022")
        .arg("--charset=utf8")
        .arg(format!("--outfile={}", out_bundle.display()))
        .output()
        .map_err(|err| format!("failed to run {}: {err}", esbuild_path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("esbuild failed ({}): {}", output.status, stderr.trim()).into());
    }

    Ok(())
}

/// Emits Cargo change-detection directives for npm metadata and every file under `ts/`.
fn emit_rerun_directives(manifest_dir: &Path) -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=package.json");
    println!("cargo:rerun-if-changed=package-lock.json");
    println!("cargo:rerun-if-changed=tsconfig.json");
    println!("cargo:rerun-if-changed=ts");

    let ts_dir = manifest_dir.join("ts");
    let mut ts_entries = Vec::new();
    for entry in fs::read_dir(&ts_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("ts")
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            ts_entries.push(name.to_string());
        }
    }
    ts_entries.sort();
    for name in ts_entries {
        println!("cargo:rerun-if-changed=ts/{name}");
    }
    Ok(())
}
