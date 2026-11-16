// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Collects third-party license notices for locked Rust crate dependencies.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::json::{JsonValue, parse_json};

/// Extracted crate metadata needed for license aggregation.
struct CratePackage {
    /// Unique Cargo package identifier string.
    id: String,
    /// Crate name.
    name: String,
    /// Crate semver version string.
    version: String,
    /// SPDX license expression declared in `Cargo.toml`.
    license: String,
    /// Optional explicit `license-file` path relative to crate root.
    license_file: Option<String>,
    /// Optional upstream repository URL.
    repository: String,
    /// Directory holding the crate's `Cargo.toml`.
    manifest_dir: PathBuf,
}

/// Collects license notices from `Cargo.lock` dependencies and writes them to stdout.
pub(crate) fn run_collect_licenses(root: &Path) -> Result<(), String> {
    let raw_meta = fetch_cargo_metadata(root)?;
    let doc = parse_json(&raw_meta)?;

    let workspace_members: BTreeSet<String> = doc
        .get("workspace_members")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    let packages_arr = doc
        .get("packages")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "cargo metadata JSON has no `packages` array".to_owned())?;

    let mut external = Vec::new();
    for item in packages_arr {
        let Some(pkg) = parse_package(item)? else {
            continue;
        };
        if !workspace_members.contains(&pkg.id) {
            external.push(pkg);
        }
    }

    external.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));

    let root_license = fs::read_to_string(root.join("LICENSE"))
        .map_err(|err| format!("LICENSE: {err}"))?
        .replace("\r\n", "\n");

    let mut sections = vec![
        concat!(
            "flow-music-mcp: licenses and source locations\n",
            "This file includes all locked Rust dependencies. ",
            "It also includes build dependencies and targets for other platforms.\n"
        )
        .to_owned(),
        format!("flow-music-mcp\n{root_license}"),
    ];

    for pkg in external {
        sections.push(build_package_section(&pkg)?);
    }

    let output = sections.join(
        "\n================================================================================\n\n",
    );
    print!("{output}");
    Ok(())
}

/// Runs `cargo metadata` using `--locked` (trying `--offline` first).
fn fetch_cargo_metadata(root: &Path) -> Result<String, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let offline_out = Command::new(&cargo)
        .args(["metadata", "--locked", "--offline", "--format-version=1"])
        .current_dir(root)
        .output()
        .map_err(|err| format!("Failed to run cargo metadata: {err}"))?;
    if offline_out.status.success() {
        return String::from_utf8(offline_out.stdout)
            .map_err(|err| format!("Invalid UTF-8 in cargo metadata: {err}"));
    }

    let online_out = Command::new(&cargo)
        .args(["metadata", "--locked", "--format-version=1"])
        .current_dir(root)
        .output()
        .map_err(|err| format!("Failed to run cargo metadata: {err}"))?;
    if !online_out.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&online_out.stderr)
        ));
    }
    String::from_utf8(online_out.stdout)
        .map_err(|err| format!("Invalid UTF-8 in cargo metadata: {err}"))
}

/// Converts one JSON element in `packages` into [`CratePackage`].
fn parse_package(item: &JsonValue) -> Result<Option<CratePackage>, String> {
    let Some(id) = item.get("id").and_then(JsonValue::as_str) else {
        return Ok(None);
    };
    let Some(name) = item.get("name").and_then(JsonValue::as_str) else {
        return Ok(None);
    };
    let Some(version) = item.get("version").and_then(JsonValue::as_str) else {
        return Ok(None);
    };
    let Some(manifest_path) = item.get("manifest_path").and_then(JsonValue::as_str) else {
        return Ok(None);
    };
    let manifest_dir = Path::new(manifest_path)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("Invalid manifest_path: {manifest_path}"))?;

    let license = item
        .get("license")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_owned();
    let license_file = item
        .get("license_file")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    let repository = item
        .get("repository")
        .and_then(JsonValue::as_str)
        .unwrap_or("see source archive")
        .to_owned();

    Ok(Some(CratePackage {
        id: id.to_owned(),
        name: name.to_owned(),
        version: version.to_owned(),
        license,
        license_file,
        repository,
        manifest_dir,
    }))
}

/// Formats metadata and license texts for a single external crate.
fn build_package_section(pkg: &CratePackage) -> Result<String, String> {
    let mut license_files = BTreeSet::new();
    find_license_files(&pkg.manifest_dir, &mut license_files)?;

    if let Some(ref extra_rel) = pkg.license_file {
        let extra = pkg.manifest_dir.join(extra_rel);
        if extra.is_file() {
            license_files.insert(extra);
        }
    }

    let mut section = format!(
        "{} {}\nLicense: {}\nRepository: {}\nSource: https://crates.io/api/v1/crates/{}/{}/download\n",
        pkg.name, pkg.version, pkg.license, pkg.repository, pkg.name, pkg.version
    );

    if !license_files.is_empty() {
        for file_path in license_files {
            let relative = file_path
                .strip_prefix(&pkg.manifest_dir)
                .unwrap_or(&file_path)
                .to_string_lossy()
                .replace('\\', "/");
            let content = fs::read_to_string(&file_path)
                .map_err(|err| format!("Failed to read {}: {err}", file_path.display()))?
                .replace("\r\n", "\n");
            section.push_str(&format!("\n--- {relative} ---\n{content}"));
        }
    } else if !pkg.license.is_empty() {
        section.push_str(&format!(
            "\n(No bundled license text file found in crate archive; declared SPDX license: {})\n",
            pkg.license
        ));
    }

    Ok(section)
}

/// Scans `dir` top-level entries for `LICENSE*`, `COPYING*`, or `NOTICE*` text files.
fn find_license_files(dir: &Path, out: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("{}: {err}", dir.display()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let upper = name.to_ascii_uppercase();
        if upper.starts_with("LICENSE")
            || upper.starts_with("LICENCE")
            || upper.starts_with("COPYING")
            || upper.starts_with("NOTICE")
        {
            out.insert(path);
        }
    }
    Ok(())
}
