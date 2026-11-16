// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Builds the `@flow-music-mcp/cli` npm package and six platform packages from release binaries.

use std::fs;
use std::path::{Path, PathBuf};

use crate::json::{JsonValue, format_json_pretty, parse_json};
use crate::workspace::{cargo_package_version, rel_display};

/// npm scope shared by the CLI entrypoint and all platform binary packages.
const SCOPE: &str = "@flow-music-mcp";

/// Directory containing the template CLI launcher package relative to workspace root.
const MAIN_DIR: &str = "npm/cli";

/// Launcher script path relative to the main package root.
const LAUNCHER: &str = "bin/flow-music-mcp.js";

/// Target platform metadata mapping one release executable to one npm package.
struct Platform {
    /// Package suffix after `@flow-music-mcp/`.
    suffix: &'static str,
    /// Release executable file name inside `--dist`.
    artifact: &'static str,
    /// Value written to the `os` field in `package.json`.
    os: &'static str,
    /// Value written to the `cpu` field in `package.json`.
    cpu: &'static str,
    /// Optional `libc` requirement for Linux GNU targets.
    libc: Option<&'static str>,
}

impl Platform {
    /// Returns the full scoped npm package name.
    fn package(&self) -> String {
        format!("{SCOPE}/{}", self.suffix)
    }

    /// Returns the installed binary file name inside the platform package `bin/` directory.
    fn executable(&self) -> &'static str {
        if self.os == "win32" {
            "flow-music-mcp.exe"
        } else {
            "flow-music-mcp"
        }
    }
}

/// Six target platforms packaged alongside `@flow-music-mcp/cli`.
const PLATFORMS: [Platform; 6] = [
    Platform {
        suffix: "darwin-arm64",
        artifact: "flow-music-mcp-macos-arm64",
        os: "darwin",
        cpu: "arm64",
        libc: None,
    },
    Platform {
        suffix: "darwin-x64",
        artifact: "flow-music-mcp-macos-x86_64",
        os: "darwin",
        cpu: "x64",
        libc: None,
    },
    Platform {
        suffix: "linux-arm64",
        artifact: "flow-music-mcp-linux-arm64",
        os: "linux",
        cpu: "arm64",
        libc: Some("glibc"),
    },
    Platform {
        suffix: "linux-x64",
        artifact: "flow-music-mcp-linux-x86_64",
        os: "linux",
        cpu: "x64",
        libc: Some("glibc"),
    },
    Platform {
        suffix: "win32-arm64",
        artifact: "flow-music-mcp-windows-arm64.exe",
        os: "win32",
        cpu: "arm64",
        libc: None,
    },
    Platform {
        suffix: "win32-x64",
        artifact: "flow-music-mcp-windows-x86_64.exe",
        os: "win32",
        cpu: "x64",
        libc: None,
    },
];

/// Parses `--dist <PATH>` and `--out <PATH>` arguments and runs [`pack`].
pub(crate) fn run_npm_pack(root: &Path, args: &[String]) -> Result<(), String> {
    let mut dist: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut i = 0usize;
    while i < args.len() {
        let flag = args[i].as_str();
        let next = args.get(i + 1);
        match flag {
            "--dist" => {
                let Some(val) = next else {
                    return Err("Missing path after --dist".to_owned());
                };
                dist = Some(PathBuf::from(val));
                i += 2;
            }
            "--out" => {
                let Some(val) = next else {
                    return Err("Missing path after --out".to_owned());
                };
                out = Some(PathBuf::from(val));
                i += 2;
            }
            other => {
                return Err(format!("Unknown argument for npm-pack: {other}"));
            }
        }
    }

    let dist_dir = dist.ok_or_else(|| "npm-pack requires --dist <PATH>".to_owned())?;
    let out_dir = out.ok_or_else(|| "npm-pack requires --out <PATH>".to_owned())?;
    pack(root, &dist_dir, &out_dir)
}

/// Packages release binaries in `dist` and the CLI launcher into `out`.
fn pack(root: &Path, dist: &Path, out: &Path) -> Result<(), String> {
    let template_path = root.join(MAIN_DIR).join("package.json");
    let template_raw = fs::read_to_string(&template_path)
        .map_err(|err| format!("{}: {err}", template_path.display()))?;
    let template = parse_json(&template_raw)
        .map_err(|err| format!("{}: {err}", rel_display(root, &template_path)))?;
    let version = template
        .get("version")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{MAIN_DIR}/package.json has no version string"))?;

    let cargo_raw =
        fs::read_to_string(root.join("Cargo.toml")).map_err(|err| format!("Cargo.toml: {err}"))?;
    let cargo_version = cargo_package_version(&cargo_raw)?;
    if version != cargo_version {
        return Err(format!(
            "{MAIN_DIR}/package.json has version {version}, but Cargo.toml has {cargo_version}"
        ));
    }

    prepare_out(out)?;

    let licenses = [
        (root.join("LICENSE"), "LICENSE"),
        (
            dist.join("THIRD_PARTY_LICENSES.txt"),
            "THIRD_PARTY_LICENSES.txt",
        ),
    ];

    for platform in &PLATFORMS {
        let source = dist.join(platform.artifact);
        if !source.is_file() {
            return Err(format!("{}: the executable is absent", source.display()));
        }
        let directory = out.join(platform.suffix);
        let bin = directory.join("bin");
        create_dir(&bin)?;
        let target = bin.join(platform.executable());
        copy_file(&source, &target)?;
        make_executable(&target)?;
        for (license, name) in &licenses {
            if license.is_file() {
                copy_file(license, &directory.join(name))?;
            }
        }
        write_json(
            &directory.join("package.json"),
            &platform_manifest(platform, &template, &version),
        )?;
        println!(
            "{}@{version}: {}",
            platform.package(),
            rel_display(root, &directory)
        );
    }

    let cli_dir = out.join("cli");
    create_dir(&cli_dir.join("bin"))?;
    let launcher_dst = cli_dir.join(LAUNCHER);
    copy_file(&root.join(MAIN_DIR).join(LAUNCHER), &launcher_dst)?;
    make_executable(&launcher_dst)?;
    copy_file(&root.join("README.md"), &cli_dir.join("README.md"))?;
    copy_file(&root.join("LICENSE"), &cli_dir.join("LICENSE"))?;
    write_json(
        &cli_dir.join("package.json"),
        &main_manifest(template, &version)?,
    )?;
    println!("{SCOPE}/cli@{version}: {}", rel_display(root, &cli_dir));
    Ok(())
}

/// Builds the main `@flow-music-mcp/cli` manifest with synchronized `optionalDependencies`.
fn main_manifest(mut template: JsonValue, version: &str) -> Result<JsonValue, String> {
    let deps: Vec<(String, JsonValue)> = PLATFORMS
        .iter()
        .map(|platform| (platform.package(), JsonValue::Str(version.to_owned())))
        .collect();
    let entries = template
        .as_object_mut()
        .ok_or_else(|| format!("{MAIN_DIR}/package.json must be a JSON object"))?;
    JsonValue::set_object_key(entries, "version", JsonValue::Str(version.to_owned()));
    JsonValue::set_object_key(entries, "optionalDependencies", JsonValue::Object(deps));
    Ok(template)
}

/// Builds the `package.json` manifest for one platform executable package.
fn platform_manifest(platform: &Platform, template: &JsonValue, version: &str) -> JsonValue {
    let mut entries = vec![
        ("name".to_owned(), JsonValue::Str(platform.package())),
        ("version".to_owned(), JsonValue::Str(version.to_owned())),
        (
            "description".to_owned(),
            JsonValue::Str(format!(
                "The flow-music-mcp executable for {} {}.",
                platform.os, platform.cpu
            )),
        ),
        (
            "license".to_owned(),
            template.get("license").cloned().unwrap_or(JsonValue::Null),
        ),
    ];

    for key in ["repository", "homepage", "bugs"] {
        if let Some(val) = template.get(key) {
            entries.push((key.to_owned(), val.clone()));
        }
    }

    entries.push((
        "os".to_owned(),
        JsonValue::Array(vec![JsonValue::Str(platform.os.to_owned())]),
    ));
    entries.push((
        "cpu".to_owned(),
        JsonValue::Array(vec![JsonValue::Str(platform.cpu.to_owned())]),
    ));
    if let Some(libc) = platform.libc {
        entries.push((
            "libc".to_owned(),
            JsonValue::Array(vec![JsonValue::Str(libc.to_owned())]),
        ));
    }
    entries.push((
        "files".to_owned(),
        JsonValue::Array(vec![
            JsonValue::Str("bin".to_owned()),
            JsonValue::Str("LICENSE*".to_owned()),
            JsonValue::Str("THIRD_PARTY_LICENSES.txt".to_owned()),
        ]),
    ));
    entries.push(("preferUnplugged".to_owned(), JsonValue::Bool(true)));
    JsonValue::Object(entries)
}

/// Ensures `out` exists and does not contain pre-existing entries.
fn prepare_out(out: &Path) -> Result<(), String> {
    if out.exists() {
        let mut entries = fs::read_dir(out).map_err(|err| format!("{}: {err}", out.display()))?;
        if entries.next().is_some() {
            return Err(format!("{}: the directory is not empty", out.display()));
        }
    }
    create_dir(out)
}

/// Writes `value` as 2-space formatted JSON to `path`.
fn write_json(path: &Path, value: &JsonValue) -> Result<(), String> {
    let text = format_json_pretty(value);
    fs::write(path, text).map_err(|err| format!("{}: {err}", path.display()))
}

/// Recursively creates `path` and any missing parent directories.
fn create_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| format!("{}: {err}", path.display()))
}

/// Copies a regular file from `source` to `target`.
fn copy_file(source: &Path, target: &Path) -> Result<(), String> {
    fs::copy(source, target)
        .map(|_| ())
        .map_err(|err| format!("{} -> {}: {err}", source.display(), target.display()))
}

/// Sets Unix executable mode `0755` on `path`.
fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|err| format!("{}: {err}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
