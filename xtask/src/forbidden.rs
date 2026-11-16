// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Forbidden test artifact, attribute, manifest, layout, and TypeScript checks.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::lexer::{RustToken, TokenKind, lex_rust};
use crate::workspace::{WorkspaceFiles, cargo_package_version, rel_display};

/// Required npm platform package dependencies in `npm/cli/package.json`.
const REQUIRED_NPM_PLATFORMS: &[&str] = &[
    "@flow-music-mcp/darwin-arm64",
    "@flow-music-mcp/darwin-x64",
    "@flow-music-mcp/linux-arm64",
    "@flow-music-mcp/linux-x64",
    "@flow-music-mcp/win32-arm64",
    "@flow-music-mcp/win32-x64",
];

/// Forbidden Rust attribute names or path segments.
const FORBIDDEN_ATTRS: &[&str] = &[
    "test",
    "bench",
    "rstest",
    "test_case",
    "proptest",
    "quickcheck",
    "should_panic",
    "allow",
    "expect",
];

/// Forbidden test script or test package strings inside `package.json`.
const FORBIDDEN_NPM_TOKENS: &[&str] = &[
    "test",
    "jest",
    "vitest",
    "mocha",
    "ava",
    "tap",
    "jasmine",
    "cypress",
    "playwright",
    "puppeteer",
];

/// Verifies zero-test rules, flat layout, manifest hygiene, and TypeScript bundle checks.
pub(crate) fn check_forbidden_code(root: &Path, files: &WorkspaceFiles) -> Vec<String> {
    let mut errors = Vec::new();

    for dir in &files.test_dirs {
        errors.push(format!(
            "{}: forbidden test or benchmark directory",
            rel_display(root, dir)
        ));
    }

    for dir in &files.nested_source_dirs {
        errors.push(format!(
            "{}: subdirectories are forbidden inside flat source directories",
            rel_display(root, dir)
        ));
    }

    if root.join("src/lib.rs").exists() {
        errors.push("src/lib.rs: forbidden library root in binary-only repository".to_string());
    }

    let mut runtime_embed_count = 0usize;
    for path in &files.rust_files {
        let Ok(src) = fs::read_to_string(path) else {
            continue;
        };
        let lex = lex_rust(&src);
        let label = rel_display(root, path);
        scan_rust_tokens(&label, &lex.tokens, &mut errors);
        if label.starts_with("src/") {
            scan_include_str(&label, &lex.tokens, &mut runtime_embed_count, &mut errors);
        }
    }
    let expected_embeds = usize::from(root.join("src/engine.rs").is_file());
    if runtime_embed_count != expected_embeds {
        errors.push(format!(
            "src/: expected {expected_embeds} music_runtime.js include_str! embed(s), found {runtime_embed_count}"
        ));
    }

    for manifest in &files.cargo_manifests {
        let Ok(raw) = fs::read_to_string(manifest) else {
            continue;
        };
        let label = rel_display(root, manifest);
        scan_cargo_manifest(&label, &raw, &mut errors);
    }

    check_ts_project(root, files, &mut errors);
    check_release_version_sync(root, &mut errors);
    errors
}

/// Scans tokenized Rust source for forbidden attributes, `cfg(test)`, and `async_trait`.
fn scan_rust_tokens(label: &str, tokens: &[RustToken], errors: &mut Vec<String>) {
    for tok in tokens {
        if tok.kind == TokenKind::Ident && tok.text == "async_trait" {
            errors.push(format!(
                "{label}:{}: forbidden `async_trait` token",
                tok.line
            ));
        }
    }

    let mut i = 0usize;
    while i < tokens.len() {
        let is_attr = tokens[i].kind == TokenKind::Punct
            && tokens[i].text == "#"
            && ((i + 1 < tokens.len()
                && tokens[i + 1].kind == TokenKind::Punct
                && tokens[i + 1].text == "[")
                || (i + 2 < tokens.len()
                    && tokens[i + 1].kind == TokenKind::Punct
                    && tokens[i + 1].text == "!"
                    && tokens[i + 2].kind == TokenKind::Punct
                    && tokens[i + 2].text == "["));
        if !is_attr {
            i += 1;
            continue;
        }
        let open_bracket = if tokens[i + 1].text == "[" {
            i + 1
        } else {
            i + 2
        };
        let Some(close_bracket) = find_matching_pair(tokens, open_bracket, "[", "]") else {
            break;
        };
        check_attribute_slice(label, &tokens[open_bracket + 1..close_bracket], errors);
        i = close_bracket + 1;
    }
}

/// Inspects the token slice inside `#[...]` or `#![...]` for forbidden attributes.
fn check_attribute_slice(label: &str, slice: &[RustToken], errors: &mut Vec<String>) {
    let Some(first) = slice.first() else {
        return;
    };
    if first.kind != TokenKind::Ident {
        return;
    }
    let mut path_segments = Vec::new();
    let mut idx = 0usize;
    while idx < slice.len() {
        if slice[idx].kind == TokenKind::Ident {
            path_segments.push(slice[idx].text.as_str());
            if idx + 2 < slice.len()
                && slice[idx + 1].kind == TokenKind::Punct
                && slice[idx + 1].text == ":"
                && slice[idx + 2].kind == TokenKind::Punct
                && slice[idx + 2].text == ":"
            {
                idx += 3;
                continue;
            }
            idx += 1;
        }
        break;
    }

    for seg in &path_segments {
        if FORBIDDEN_ATTRS.contains(seg) {
            errors.push(format!(
                "{label}:{}: forbidden attribute `#{}`",
                first.line,
                path_segments.join("::")
            ));
            return;
        }
    }

    if path_segments.as_slice() == ["cfg"]
        && idx < slice.len()
        && slice[idx].kind == TokenKind::Punct
        && slice[idx].text == "("
    {
        if slice[idx..]
            .iter()
            .any(|t| t.kind == TokenKind::Ident && t.text == "test")
        {
            errors.push(format!(
                "{label}:{}: forbidden `#[cfg(test)]` attribute",
                first.line
            ));
        }
    } else if path_segments.as_slice() == ["cfg_attr"]
        && idx < slice.len()
        && slice[idx].kind == TokenKind::Punct
        && slice[idx].text == "("
    {
        let before_comma = slice[idx + 1..]
            .iter()
            .take_while(|t| !(t.kind == TokenKind::Punct && t.text == ","));
        if before_comma
            .filter(|t| t.kind == TokenKind::Ident)
            .any(|t| t.text == "test")
        {
            errors.push(format!(
                "{label}:{}: forbidden `#[cfg_attr(test, ...)]` attribute",
                first.line
            ));
        }
    }
}

/// Finds the matching closing punctuation index for `open_tok`/`close_tok`.
fn find_matching_pair(
    tokens: &[RustToken],
    open_idx: usize,
    open_tok: &str,
    close_tok: &str,
) -> Option<usize> {
    let mut depth = 0usize;
    for (idx, tok) in tokens.iter().enumerate().skip(open_idx) {
        if tok.kind == TokenKind::Punct && tok.text == open_tok {
            depth += 1;
        } else if tok.kind == TokenKind::Punct && tok.text == close_tok {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(idx);
            }
        }
    }
    None
}

/// Scans `include_str!(...)` calls in `src/*.rs` for valid single bundle embedding and forbidden per-file paths.
fn scan_include_str(
    label: &str,
    tokens: &[RustToken],
    runtime_embed_count: &mut usize,
    errors: &mut Vec<String>,
) {
    let mut i = 0usize;
    while i + 2 < tokens.len() {
        if tokens[i].kind == TokenKind::Ident
            && tokens[i].text == "include_str"
            && tokens[i + 1].kind == TokenKind::Punct
            && tokens[i + 1].text == "!"
            && tokens[i + 2].kind == TokenKind::Punct
            && tokens[i + 2].text == "("
            && let Some(close_paren) = find_matching_pair(tokens, i + 2, "(", ")")
        {
            for tok in &tokens[i + 3..close_paren] {
                if tok.kind != TokenKind::Literal {
                    continue;
                }
                let normalized = tok.text.replace('\\', "/");
                if normalized.contains("../js/") || normalized.contains("../ts/") {
                    errors.push(format!(
                        "{label}:{}: forbidden direct script include_str! path `{}`",
                        tok.line, tok.text
                    ));
                }
                if normalized == "music_runtime.js" || normalized.ends_with("/music_runtime.js") {
                    *runtime_embed_count += 1;
                }
            }
            i = close_paren + 1;
            continue;
        }
        i += 1;
    }
}

/// Scans a `Cargo.toml` manifest for banned test sections, workspace deps, and `async-trait`.
fn scan_cargo_manifest(label: &str, raw: &str, errors: &mut Vec<String>) {
    let is_xtask = label == "xtask/Cargo.toml";
    let mut in_xtask_deps = false;

    for (idx, raw_line) in raw.lines().enumerate() {
        let line_no = idx + 1;
        let stripped = strip_toml_comment(raw_line);
        let trimmed = stripped.trim();
        if trimmed.is_empty() {
            continue;
        }
        let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
        let lower = compact.to_ascii_lowercase();

        if trimmed.starts_with('[') {
            in_xtask_deps = is_xtask
                && (lower == "[dependencies]"
                    || lower.starts_with("[dependencies.")
                    || lower.contains(".dependencies]"));
            if lower.contains("dev-dependencies")
                || lower == "[[test]]"
                || lower == "[[bench]]"
                || lower == "[workspace.dependencies]"
                || lower.starts_with("[workspace.dependencies.")
            {
                errors.push(format!("{label}:{line_no}: forbidden section `{trimmed}`"));
            }
            continue;
        }

        if in_xtask_deps {
            errors.push(format!(
                "{label}:{line_no}: xtask must keep [dependencies] empty"
            ));
        }
        if lower.contains("async-trait") || lower.contains("async_trait") {
            errors.push(format!("{label}:{line_no}: forbidden `async-trait` entry"));
        }
        if lower == "test=true" || lower == "doctest=true" || lower == "bench=true" {
            errors.push(format!("{label}:{line_no}: forbidden test harness flag"));
        }
    }
}

/// Removes trailing `#` comments outside quoted TOML string literals.
fn strip_toml_comment(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        if in_double {
            out.push(ch);
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
            continue;
        }
        if in_single {
            out.push(ch);
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_double = true;
                out.push(ch);
            }
            '\'' => {
                in_single = true;
                out.push(ch);
            }
            '#' => break,
            other => out.push(other),
        }
    }
    out
}

/// Extracts decoded double-quoted JSON string literals from `raw`.
fn extract_json_strings(raw: &str) -> Vec<String> {
    let chars: Vec<char> = raw.chars().collect();
    let mut strings = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] != '"' {
            i += 1;
            continue;
        }
        let Some((val, next_idx)) = read_json_string(&chars, i) else {
            break;
        };
        strings.push(val);
        i = next_idx;
    }
    strings
}

/// Verifies TypeScript layout, absence of stray `.js` files, `package.json` zero-test rules, and `tsc --noEmit`.
fn check_ts_project(root: &Path, files: &WorkspaceFiles, errors: &mut Vec<String>) {
    if root.join("js").exists() {
        errors.push("js/: legacy JavaScript directory must be removed".to_string());
    }
    for path in &files.stray_script_files {
        errors.push(format!(
            "{}: forbidden stray JavaScript or misplaced TypeScript file",
            rel_display(root, path)
        ));
    }
    if files.ts_files.is_empty() || !root.join("ts/index.ts").is_file() {
        errors.push("ts/index.ts: missing TypeScript bundle entrypoint".to_string());
    }

    let pkg_path = root.join("package.json");
    scan_npm_manifest(root, &pkg_path, errors);
    let cli_pkg_path = root.join("npm/cli/package.json");
    if cli_pkg_path.is_file() {
        scan_npm_manifest(root, &cli_pkg_path, errors);
    }

    let tsc_bin = if cfg!(windows) { "tsc.cmd" } else { "tsc" };
    let tsc_path = root.join("node_modules").join(".bin").join(tsc_bin);
    if !tsc_path.is_file() {
        errors.push(format!(
            "{}: missing local TypeScript compiler; run `npm ci` first",
            rel_display(root, &tsc_path)
        ));
        return;
    }

    match Command::new(&tsc_path)
        .current_dir(root)
        .args(["--noEmit", "--project", "tsconfig.json"])
        .output()
    {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = [stdout.trim(), stderr.trim()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            errors.push(format!("tsc --noEmit failed:\n{combined}"));
        }
        Err(err) => {
            errors.push(format!(
                "failed to run {}: {err}",
                rel_display(root, &tsc_path)
            ));
        }
    }
}

/// Scans an npm `package.json` manifest for forbidden test scripts or test packages.
fn scan_npm_manifest(root: &Path, pkg_path: &Path, errors: &mut Vec<String>) {
    let label = rel_display(root, pkg_path);
    match fs::read_to_string(pkg_path) {
        Ok(raw) => {
            for val in extract_json_strings(&raw) {
                let lower = val.to_ascii_lowercase();
                if FORBIDDEN_NPM_TOKENS.contains(&lower.as_str()) {
                    errors.push(format!(
                        "{label}: forbidden test script or package string `{val}`"
                    ));
                }
            }
        }
        Err(err) => {
            errors.push(format!("{label}: failed to read manifest: {err}"));
        }
    }
}

/// Extracts string entries mapped by `"key": "value"` in `raw`.
fn extract_json_kv_pairs(raw: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = raw.chars().collect();
    let mut pairs = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] != '"' {
            i += 1;
            continue;
        }
        let Some((key, after_key)) = read_json_string(&chars, i) else {
            break;
        };
        let mut j = after_key;
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        if j < chars.len() && chars[j] == ':' {
            j += 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len()
                && chars[j] == '"'
                && let Some((val, after_val)) = read_json_string(&chars, j)
            {
                pairs.push((key, val));
                i = after_val;
                continue;
            }
        }
        i = after_key;
    }
    pairs
}

/// Reads one double-quoted JSON string starting at `start_idx` and returns `(decoded, next_idx)`.
fn read_json_string(chars: &[char], start_idx: usize) -> Option<(String, usize)> {
    if chars.get(start_idx).copied() != Some('"') {
        return None;
    }
    let mut out = String::new();
    let mut i = start_idx + 1;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '"' {
            return Some((out, i + 1));
        }
        out.push(c);
        i += 1;
    }
    None
}

/// Verifies matching release versions across Cargo manifests, release-please, and `npm/cli`.
fn check_release_version_sync(root: &Path, errors: &mut Vec<String>) {
    let cli_pkg_path = root.join("npm/cli/package.json");
    if !cli_pkg_path.is_file() {
        return;
    }

    let root_cargo = match fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|e| e.to_string())
        .and_then(|s| cargo_package_version(&s))
    {
        Ok(v) => v,
        Err(err) => {
            errors.push(format!("Cargo.toml: {err}"));
            return;
        }
    };

    match fs::read_to_string(root.join("xtask/Cargo.toml"))
        .map_err(|e| e.to_string())
        .and_then(|s| cargo_package_version(&s))
    {
        Ok(v) if v != root_cargo => errors.push(format!(
            "xtask/Cargo.toml: version `{v}` does not match Cargo.toml `{root_cargo}`"
        )),
        Err(err) => errors.push(format!("xtask/Cargo.toml: {err}")),
        Ok(_) => {}
    }

    match fs::read_to_string(root.join(".release-please-manifest.json")) {
        Ok(raw) => {
            let pairs = extract_json_kv_pairs(&raw);
            let val = pairs
                .iter()
                .find(|(k, _)| k == ".")
                .map(|(_, v)| v.as_str());
            if val != Some(root_cargo.as_str()) {
                errors.push(format!(
                    ".release-please-manifest.json: expected `.` = `{root_cargo}`"
                ));
            }
        }
        Err(err) => errors.push(format!(".release-please-manifest.json: {err}")),
    }

    match fs::read_to_string(&cli_pkg_path) {
        Ok(raw) => {
            let pairs = extract_json_kv_pairs(&raw);
            let pkg_ver = pairs
                .iter()
                .find(|(k, _)| k == "version")
                .map(|(_, v)| v.as_str());
            if pkg_ver != Some(root_cargo.as_str()) {
                errors.push(format!(
                    "npm/cli/package.json: version `{}` does not match `{root_cargo}`",
                    pkg_ver.unwrap_or("missing")
                ));
            }
            for dep in REQUIRED_NPM_PLATFORMS {
                let dep_ver = pairs
                    .iter()
                    .find(|(k, _)| k == dep)
                    .map(|(_, v)| v.as_str());
                if dep_ver != Some(root_cargo.as_str()) {
                    errors.push(format!(
                        "npm/cli/package.json: optionalDependencies[`{dep}`] = `{}` does not match `{root_cargo}`",
                        dep_ver.unwrap_or("missing")
                    ));
                }
            }
        }
        Err(err) => errors.push(format!("npm/cli/package.json: {err}")),
    }
}
