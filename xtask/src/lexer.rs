// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Finite-state Rust lexer and TypeScript code-line scanner.

/// Classification of a lexed Rust token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenKind {
    /// Identifier or keyword.
    Ident,
    /// Single punctuation or operator symbol.
    Punct,
    /// String, raw string, byte string, char, or numeric literal.
    Literal,
    /// Outer or inner doc comment text with its 1-based starting line.
    DocComment,
}

/// One token produced from Rust source text.
#[derive(Debug, Clone)]
pub(crate) struct RustToken {
    /// Kind of token.
    pub(crate) kind: TokenKind,
    /// Raw or extracted text of the token.
    pub(crate) text: String,
    /// 1-based starting line number.
    pub(crate) line: usize,
    /// True when a line doc comment is inner (`//!` or `/*!`).
    pub(crate) is_inner_doc: bool,
}

/// Lexed Rust source output with tokens and non-comment code line count.
pub(crate) struct RustLex {
    /// Count of lines containing non-comment, non-whitespace Rust code tokens.
    pub(crate) code_lines: usize,
    /// Ordered token stream including doc comments and literals.
    pub(crate) tokens: Vec<RustToken>,
}

/// Tokenizes Rust source `src` and counts non-comment, non-blank code lines.
pub(crate) fn lex_rust(src: &str) -> RustLex {
    let chars: Vec<char> = src.chars().collect();
    let len = chars.len();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut line_has_code = false;
    let mut code_lines = 0usize;
    let mut tokens = Vec::new();

    while i < len {
        let c = chars[i];
        if c == '\n' {
            if line_has_code {
                code_lines += 1;
            }
            line_has_code = false;
            line += 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        if c == '/' && i + 1 < len && chars[i + 1] == '/' {
            let start_line = line;
            let third = chars.get(i + 2).copied();
            let fourth = chars.get(i + 3).copied();
            let is_doc = (third == Some('/') && fourth != Some('/')) || third == Some('!');
            let is_inner = third == Some('!');
            let content_start = if is_doc { i + 3 } else { i + 2 };
            let mut end = content_start;
            while end < len && chars[end] != '\n' {
                end += 1;
            }
            if is_doc {
                let raw: String = chars[content_start..end].iter().collect();
                let cleaned = raw.strip_prefix(' ').unwrap_or(&raw).to_string();
                tokens.push(RustToken {
                    kind: TokenKind::DocComment,
                    text: cleaned,
                    line: start_line,
                    is_inner_doc: is_inner,
                });
            }
            i = end;
            continue;
        }

        if c == '/' && i + 1 < len && chars[i + 1] == '*' {
            let start_line = line;
            let third = chars.get(i + 2).copied();
            let fourth = chars.get(i + 3).copied();
            let is_doc = (third == Some('*') && fourth != Some('*') && fourth != Some('/'))
                || third == Some('!');
            let is_inner = third == Some('!');
            let body_start = if is_doc { i + 3 } else { i + 2 };
            let mut depth = 1usize;
            i += 2;
            let mut body_end = len;
            while i < len && depth > 0 {
                if chars[i] == '/' && i + 1 < len && chars[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && i + 1 < len && chars[i + 1] == '/' {
                    depth -= 1;
                    if depth == 0 {
                        body_end = i;
                    }
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        if line_has_code {
                            code_lines += 1;
                        }
                        line_has_code = false;
                        line += 1;
                    }
                    i += 1;
                }
            }
            if is_doc && body_start <= body_end {
                let raw: String = chars[body_start..body_end].iter().collect();
                tokens.push(RustToken {
                    kind: TokenKind::DocComment,
                    text: strip_block_doc_stars(&raw),
                    line: start_line,
                    is_inner_doc: is_inner,
                });
            }
            continue;
        }

        if let Some((next_i, text, end_line, spanned_nonblank)) =
            try_scan_raw_string(&chars, i, line)
        {
            if spanned_nonblank {
                line_has_code = true;
            }
            code_lines += spanned_extra_code_lines(&chars[i..next_i]);
            tokens.push(RustToken {
                kind: TokenKind::Literal,
                text,
                line,
                is_inner_doc: false,
            });
            line = end_line;
            i = next_i;
            continue;
        }

        if c == '"' || ((c == 'b' || c == 'c') && i + 1 < len && chars[i + 1] == '"') {
            let start_line = line;
            let q_start = if c == '"' { i } else { i + 1 };
            let (next_i, text, end_line) = scan_quoted_string(&chars, q_start, line);
            code_lines += spanned_extra_code_lines(&chars[i..next_i]);
            line_has_code = true;
            tokens.push(RustToken {
                kind: TokenKind::Literal,
                text,
                line: start_line,
                is_inner_doc: false,
            });
            line = end_line;
            i = next_i;
            continue;
        }

        if c == '\'' || (c == 'b' && i + 1 < len && chars[i + 1] == '\'') {
            let q_idx = if c == '\'' { i } else { i + 1 };
            if let Some((next_i, text)) = try_scan_char_literal(&chars, q_idx) {
                line_has_code = true;
                tokens.push(RustToken {
                    kind: TokenKind::Literal,
                    text,
                    line,
                    is_inner_doc: false,
                });
                i = next_i;
                continue;
            }
        }

        if c.is_ascii_digit() {
            line_has_code = true;
            let start = i;
            i += 1;
            while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(RustToken {
                kind: TokenKind::Literal,
                text: chars[start..i].iter().collect(),
                line,
                is_inner_doc: false,
            });
            continue;
        }

        if is_ident_start(c) {
            line_has_code = true;
            let start = i;
            i += 1;
            while i < len && is_ident_continue(chars[i]) {
                i += 1;
            }
            tokens.push(RustToken {
                kind: TokenKind::Ident,
                text: chars[start..i].iter().collect(),
                line,
                is_inner_doc: false,
            });
            continue;
        }

        line_has_code = true;
        tokens.push(RustToken {
            kind: TokenKind::Punct,
            text: c.to_string(),
            line,
            is_inner_doc: false,
        });
        i += 1;
    }

    if line_has_code {
        code_lines += 1;
    }

    RustLex { code_lines, tokens }
}

/// Strips optional leading `*` prefixes from block doc comment lines.
fn strip_block_doc_stars(raw: &str) -> String {
    let mut lines = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if let Some(after) = trimmed.strip_prefix('*') {
            lines.push(after.strip_prefix(' ').unwrap_or(after).to_string());
        } else {
            lines.push(line.to_string());
        }
    }
    lines.join("\n")
}

/// Counts extra non-blank lines spanned inside a multi-line literal beyond its closing line.
fn spanned_extra_code_lines(slice: &[char]) -> usize {
    let mut extra = 0usize;
    let mut cur_has_nonblank = false;
    let mut first_line = true;
    for &ch in slice {
        if ch == '\n' {
            if !first_line && cur_has_nonblank {
                extra += 1;
            }
            first_line = false;
            cur_has_nonblank = false;
        } else if !ch.is_whitespace() {
            cur_has_nonblank = true;
        }
    }
    extra
}

/// Scans a raw string (`r#"..."#`, `br#"..."#`, `cr#"..."#`) starting at `i`.
fn try_scan_raw_string(
    chars: &[char],
    i: usize,
    start_line: usize,
) -> Option<(usize, String, usize, bool)> {
    let len = chars.len();
    let r_pos = if chars[i] == 'r' {
        i
    } else if (chars[i] == 'b' || chars[i] == 'c') && i + 1 < len && chars[i + 1] == 'r' {
        i + 1
    } else {
        return None;
    };
    let mut j = r_pos + 1;
    let mut hashes = 0usize;
    while j < len && chars[j] == '#' {
        hashes += 1;
        j += 1;
    }
    if j >= len || chars[j] != '"' {
        return None;
    }
    let content_start = j + 1;
    j = content_start;
    let mut line = start_line;
    let mut has_nonblank = true;
    while j < len {
        let ch = chars[j];
        if ch == '\n' {
            line += 1;
            j += 1;
            continue;
        }
        if !ch.is_whitespace() {
            has_nonblank = true;
        }
        if ch == '"' {
            let mut matched = 0usize;
            while matched < hashes && j + 1 + matched < len && chars[j + 1 + matched] == '#' {
                matched += 1;
            }
            if matched == hashes {
                let text: String = chars[content_start..j].iter().collect();
                return Some((j + 1 + hashes, text, line, has_nonblank));
            }
        }
        j += 1;
    }
    let text: String = chars[content_start..len].iter().collect();
    Some((len, text, line, has_nonblank))
}

/// Scans a standard double-quoted string literal starting at `quote_idx`.
fn scan_quoted_string(
    chars: &[char],
    quote_idx: usize,
    start_line: usize,
) -> (usize, String, usize) {
    let len = chars.len();
    let mut j = quote_idx + 1;
    let mut line = start_line;
    let mut out = String::new();
    while j < len {
        let ch = chars[j];
        if ch == '\n' {
            line += 1;
            out.push('\n');
            j += 1;
            continue;
        }
        if ch == '\\' && j + 1 < len {
            let esc = chars[j + 1];
            match esc {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                '\n' => {
                    line += 1;
                }
                other => out.push(other),
            }
            j += 2;
            continue;
        }
        if ch == '"' {
            return (j + 1, out, line);
        }
        out.push(ch);
        j += 1;
    }
    (len, out, line)
}

/// Attempts to scan a char literal (`'a'`, `'\n'`) without matching lifetimes (`'a`).
fn try_scan_char_literal(chars: &[char], quote_idx: usize) -> Option<(usize, String)> {
    let len = chars.len();
    if quote_idx + 1 >= len {
        return None;
    }
    if chars[quote_idx + 1] == '\\' {
        let mut j = quote_idx + 2;
        if j < len {
            j += 1;
        }
        while j < len && chars[j] != '\'' && chars[j] != '\n' {
            j += 1;
        }
        if j < len && chars[j] == '\'' {
            let text: String = chars[quote_idx + 1..j].iter().collect();
            return Some((j + 1, text));
        }
        return None;
    }
    if chars[quote_idx + 1] != '\''
        && chars[quote_idx + 1] != '\n'
        && quote_idx + 2 < len
        && chars[quote_idx + 2] == '\''
    {
        return Some((quote_idx + 3, chars[quote_idx + 1].to_string()));
    }
    None
}

/// Returns `true` when `c` can start a Rust identifier.
fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

/// Returns `true` when `c` can continue a Rust identifier.
fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// Lexer mode for TypeScript code-line counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsState {
    /// Normal TypeScript code.
    Code,
    /// Single-line `//` comment.
    LineComment,
    /// Block `/* ... */` comment.
    BlockComment,
    /// Single-quoted string literal.
    SingleQuote,
    /// Double-quoted string literal.
    DoubleQuote,
    /// Template string literal body outside `${...}`.
    Template,
    /// Regular expression literal.
    Regex {
        /// True inside `[...]` character class where `/` does not close the regex.
        in_class: bool,
    },
}

/// Counts non-comment, non-blank lines in TypeScript source `src`.
pub(crate) fn count_ts_code_lines(src: &str) -> usize {
    let chars: Vec<char> = src.chars().collect();
    let len = chars.len();
    let mut i = 0usize;
    let mut state = JsState::Code;
    let mut template_braces: Vec<usize> = Vec::new();
    let mut line_has_code = false;
    let mut code_lines = 0usize;
    let mut can_start_regex = true;

    while i < len {
        let c = chars[i];
        if c == '\n' {
            if line_has_code {
                code_lines += 1;
            }
            line_has_code = false;
            match state {
                JsState::LineComment | JsState::Regex { .. } => {
                    state = JsState::Code;
                }
                JsState::Code
                | JsState::BlockComment
                | JsState::SingleQuote
                | JsState::DoubleQuote
                | JsState::Template => {}
            }
            i += 1;
            continue;
        }

        match state {
            JsState::LineComment => {
                i += 1;
            }
            JsState::BlockComment => {
                if c == '*' && i + 1 < len && chars[i + 1] == '/' {
                    state = JsState::Code;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            JsState::SingleQuote => {
                if !c.is_whitespace() {
                    line_has_code = true;
                }
                if c == '\\' && i + 1 < len {
                    i += 2;
                } else if c == '\'' {
                    state = JsState::Code;
                    can_start_regex = false;
                    i += 1;
                } else {
                    i += 1;
                }
            }
            JsState::DoubleQuote => {
                if !c.is_whitespace() {
                    line_has_code = true;
                }
                if c == '\\' && i + 1 < len {
                    i += 2;
                } else if c == '"' {
                    state = JsState::Code;
                    can_start_regex = false;
                    i += 1;
                } else {
                    i += 1;
                }
            }
            JsState::Template => {
                if !c.is_whitespace() {
                    line_has_code = true;
                }
                if c == '\\' && i + 1 < len {
                    i += 2;
                } else if c == '$' && i + 1 < len && chars[i + 1] == '{' {
                    template_braces.push(0);
                    state = JsState::Code;
                    can_start_regex = true;
                    i += 2;
                } else if c == '`' {
                    state = JsState::Code;
                    can_start_regex = false;
                    i += 1;
                } else {
                    i += 1;
                }
            }
            JsState::Regex { in_class } => {
                if !c.is_whitespace() {
                    line_has_code = true;
                }
                if c == '\\' && i + 1 < len {
                    i += 2;
                } else if c == '[' && !in_class {
                    state = JsState::Regex { in_class: true };
                    i += 1;
                } else if c == ']' && in_class {
                    state = JsState::Regex { in_class: false };
                    i += 1;
                } else if c == '/' && !in_class {
                    state = JsState::Code;
                    can_start_regex = false;
                    i += 1;
                } else {
                    i += 1;
                }
            }
            JsState::Code => {
                if c.is_whitespace() {
                    i += 1;
                    continue;
                }
                if c == '/' && i + 1 < len && chars[i + 1] == '/' {
                    state = JsState::LineComment;
                    i += 2;
                    continue;
                }
                if c == '/' && i + 1 < len && chars[i + 1] == '*' {
                    state = JsState::BlockComment;
                    i += 2;
                    continue;
                }
                line_has_code = true;
                if c == '\'' {
                    state = JsState::SingleQuote;
                    i += 1;
                } else if c == '"' {
                    state = JsState::DoubleQuote;
                    i += 1;
                } else if c == '`' {
                    state = JsState::Template;
                    i += 1;
                } else if c == '/' && can_start_regex {
                    state = JsState::Regex { in_class: false };
                    i += 1;
                } else if c == '{' {
                    if let Some(depth) = template_braces.last_mut() {
                        *depth += 1;
                    }
                    can_start_regex = true;
                    i += 1;
                } else if c == '}' {
                    if let Some(depth) = template_braces.last_mut() {
                        if *depth == 0 {
                            template_braces.pop();
                            state = JsState::Template;
                            i += 1;
                            continue;
                        }
                        *depth -= 1;
                    }
                    can_start_regex = false;
                    i += 1;
                } else if is_js_ident_start(c) {
                    let start = i;
                    i += 1;
                    while i < len && is_js_ident_continue(chars[i]) {
                        i += 1;
                    }
                    let word: String = chars[start..i].iter().collect();
                    can_start_regex = is_regex_preceding_keyword(&word);
                } else if c.is_ascii_digit() {
                    while i < len
                        && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '.' | '_'))
                    {
                        i += 1;
                    }
                    can_start_regex = false;
                } else {
                    can_start_regex = !matches!(c, ')' | ']');
                    i += 1;
                }
            }
        }
    }

    if line_has_code {
        code_lines += 1;
    }
    code_lines
}

/// Returns `true` when `c` can start a TypeScript identifier.
fn is_js_ident_start(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphabetic()
}

/// Returns `true` when `c` can continue a TypeScript identifier.
fn is_js_ident_continue(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphanumeric()
}

/// Returns `true` when a regex literal can legally follow TypeScript keyword `word`.
fn is_regex_preceding_keyword(word: &str) -> bool {
    matches!(
        word,
        "return"
            | "throw"
            | "case"
            | "else"
            | "typeof"
            | "void"
            | "delete"
            | "await"
            | "yield"
            | "in"
            | "of"
            | "new"
            | "instanceof"
            | "do"
    )
}
