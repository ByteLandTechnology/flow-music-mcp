// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Rustdoc and Markdown code-example scanner enforcing `zero-tests`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::lexer::{RustToken, TokenKind, lex_rust};
use crate::workspace::{WorkspaceFiles, rel_display};

/// Scans Rust doc comments, included doc files, and Markdown files for Rust code examples.
pub(crate) fn check_doc_examples(root: &Path, files: &WorkspaceFiles) -> Vec<String> {
    let mut errors = Vec::new();
    let mut scanned_md: HashSet<PathBuf> = HashSet::new();

    for path in &files.markdown_files {
        scanned_md.insert(path.clone());
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let label = rel_display(root, path);
        scan_markdown(&label, 1, &content, &mut errors);
    }

    for path in &files.rust_files {
        let Ok(src) = fs::read_to_string(path) else {
            continue;
        };
        let lex = lex_rust(&src);
        let label = rel_display(root, path);
        scan_rust_doc_tokens(&label, &lex.tokens, &mut errors);
        scan_doc_attributes(root, path, &lex.tokens, &mut scanned_md, &mut errors);
    }

    errors
}

/// Groups consecutive Rust doc-comment tokens into Markdown blocks and scans them.
fn scan_rust_doc_tokens(label: &str, tokens: &[RustToken], errors: &mut Vec<String>) {
    let mut idx = 0usize;
    while idx < tokens.len() {
        let tok = &tokens[idx];
        if tok.kind != TokenKind::DocComment {
            idx += 1;
            continue;
        }
        let start_line = tok.line;
        let inner = tok.is_inner_doc;
        let mut lines = vec![tok.text.clone()];
        let mut prev_line = tok.line;
        idx += 1;
        while idx < tokens.len() {
            let next = &tokens[idx];
            if next.kind == TokenKind::DocComment
                && next.is_inner_doc == inner
                && next.line == prev_line + 1
            {
                lines.push(next.text.clone());
                prev_line = next.line;
                idx += 1;
            } else {
                break;
            }
        }
        let joined = lines.join("\n");
        scan_markdown(label, start_line, &joined, errors);
    }
}

/// Scans `#[doc = "..."]` and `#[doc = include_str!("...")]` attributes in Rust token streams.
fn scan_doc_attributes(
    root: &Path,
    rust_path: &Path,
    tokens: &[RustToken],
    scanned_md: &mut HashSet<PathBuf>,
    errors: &mut Vec<String>,
) {
    let label = rel_display(root, rust_path);
    let mut i = 0usize;
    while i < tokens.len() {
        let is_attr_start = tokens[i].kind == TokenKind::Punct
            && tokens[i].text == "#"
            && ((i + 1 < tokens.len()
                && tokens[i + 1].kind == TokenKind::Punct
                && tokens[i + 1].text == "[")
                || (i + 2 < tokens.len()
                    && tokens[i + 1].kind == TokenKind::Punct
                    && tokens[i + 1].text == "!"
                    && tokens[i + 2].kind == TokenKind::Punct
                    && tokens[i + 2].text == "["));
        if !is_attr_start {
            i += 1;
            continue;
        }
        let open_bracket = if tokens[i + 1].text == "[" {
            i + 1
        } else {
            i + 2
        };
        let Some(close_bracket) = find_matching_bracket(tokens, open_bracket) else {
            break;
        };
        let inner = &tokens[open_bracket + 1..close_bracket];
        if inner.len() >= 3
            && inner[0].kind == TokenKind::Ident
            && inner[0].text == "doc"
            && inner[1].kind == TokenKind::Punct
            && inner[1].text == "="
        {
            if inner.len() == 3 && inner[2].kind == TokenKind::Literal {
                scan_markdown(&label, inner[2].line, &inner[2].text, errors);
            } else if inner.len() >= 6
                && inner[2].kind == TokenKind::Ident
                && inner[2].text == "include_str"
                && inner[3].kind == TokenKind::Punct
                && inner[3].text == "!"
                && inner[4].kind == TokenKind::Punct
                && inner[4].text == "("
                && inner[5].kind == TokenKind::Literal
                && let Some(parent) = rust_path.parent()
            {
                let target = parent.join(&inner[5].text);
                if scanned_md.insert(target.clone()) {
                    match fs::read_to_string(&target) {
                        Ok(md) => {
                            let target_label = rel_display(root, &target);
                            scan_markdown(&target_label, 1, &md, errors);
                        }
                        Err(err) => {
                            errors.push(format!(
                                "{label}:{}: failed to read included doc {}: {err}",
                                inner[5].line, inner[5].text
                            ));
                        }
                    }
                }
            }
        }
        i = close_bracket + 1;
    }
}

/// Finds the matching `]` index for an opening `[` at `open_idx`.
fn find_matching_bracket(tokens: &[RustToken], open_idx: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (idx, tok) in tokens.iter().enumerate().skip(open_idx) {
        if tok.kind == TokenKind::Punct && tok.text == "[" {
            depth += 1;
        } else if tok.kind == TokenKind::Punct && tok.text == "]" {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(idx);
            }
        }
    }
    None
}

/// Checks Markdown `content` for untagged or Rust fenced code blocks and 4-space indented blocks.
fn scan_markdown(label: &str, base_line: usize, content: &str, errors: &mut Vec<String>) {
    let mut active_fence: Option<(char, usize)> = None;
    let mut list_indent: Option<usize> = None;
    let mut prev_blank = true;

    for (offset, raw_line) in content.split('\n').enumerate() {
        let line_no = base_line + offset;
        let leading = count_leading_indent(raw_line);
        let trimmed = raw_line.trim_start();

        if let Some((f_char, f_len)) = active_fence {
            if is_closing_fence(trimmed, f_char, f_len) {
                active_fence = None;
                prev_blank = false;
            }
            continue;
        }

        if let Some((f_char, f_len, info)) = parse_opening_fence(trimmed) {
            if is_forbidden_fence_info(info) {
                errors.push(format!(
                    "{label}:{line_no}: forbidden Rust or untagged Markdown code fence"
                ));
            }
            active_fence = Some((f_char, f_len));
            prev_blank = false;
            continue;
        }

        if trimmed.is_empty() {
            prev_blank = true;
            continue;
        }

        if let Some(marker_len) = list_marker_len(trimmed) {
            let content_base = list_indent.unwrap_or(0);
            if prev_blank && leading >= content_base + 4 {
                errors.push(format!(
                    "{label}:{line_no}: forbidden 4-space indented code block"
                ));
            } else {
                list_indent = Some(leading + marker_len);
            }
            prev_blank = false;
            continue;
        }

        if let Some(li) = list_indent
            && prev_blank
            && leading < li
        {
            list_indent = None;
        }

        let content_base = list_indent.unwrap_or(0);
        if prev_blank && leading >= content_base + 4 {
            errors.push(format!(
                "{label}:{line_no}: forbidden 4-space indented code block"
            ));
        }
        prev_blank = false;
    }
}

/// Counts leading spaces on a line, expanding leading tabs to 4 spaces.
fn count_leading_indent(line: &str) -> usize {
    let mut count = 0usize;
    for ch in line.chars() {
        match ch {
            ' ' => count += 1,
            '\t' => count += 4,
            _ => break,
        }
    }
    count
}

/// Parses an opening CommonMark fence (` ``` ` or `~~~`) and its trimmed info string.
fn parse_opening_fence(trimmed: &str) -> Option<(char, usize, &str)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|&c| c == first).count();
    if len < 3 {
        return None;
    }
    let info = trimmed[len..].trim();
    if first == '`' && info.contains('`') {
        return None;
    }
    Some((first, len, info))
}

/// Returns `true` when `trimmed` closes an active code fence.
fn is_closing_fence(trimmed: &str, fence_char: char, min_len: usize) -> bool {
    let len = trimmed.chars().take_while(|&c| c == fence_char).count();
    len >= min_len && trimmed[len..].trim().is_empty()
}

/// Returns `true` when a Markdown code fence triggers rustdoc compilation or lacks a language tag.
fn is_forbidden_fence_info(info: &str) -> bool {
    if info.is_empty() {
        return true;
    }
    let first = info
        .split(|c: char| c.is_whitespace() || c == ',')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    first.is_empty()
        || matches!(
            first.as_str(),
            "rust" | "rs" | "no_run" | "ignore" | "should_panic" | "compile_fail"
        )
        || first.starts_with("edition")
}

/// Returns the byte length of a leading Markdown list marker plus trailing spaces, if present.
fn list_marker_len(trimmed: &str) -> Option<usize> {
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && matches!(bytes[0], b'-' | b'*' | b'+') && bytes[1] == b' ' {
        let spaces = bytes[1..].iter().take_while(|&&b| b == b' ').count();
        return Some(1 + spaces);
    }
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0
        && digits < 10
        && bytes.len() > digits + 1
        && matches!(bytes[digits], b'.' | b')')
        && bytes[digits + 1] == b' '
    {
        let spaces = bytes[digits + 1..]
            .iter()
            .take_while(|&&b| b == b' ')
            .count();
        return Some(digits + 1 + spaces);
    }
    None
}
