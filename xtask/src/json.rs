// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Minimal zero-dependency JSON parser and pretty formatter for `xtask`.

use std::fmt::Write as _;

/// Ordered JSON value representation preserving object key insertion order.
#[derive(Debug, Clone)]
pub(crate) enum JsonValue {
    /// JSON `null` literal.
    Null,
    /// JSON `true` or `false` literal.
    Bool(bool),
    /// Raw JSON numeric literal text.
    Number(String),
    /// Decoded JSON UTF-8 string.
    Str(String),
    /// Ordered JSON array.
    Array(Vec<JsonValue>),
    /// Ordered JSON object entries.
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    /// Returns the inner string slice when this value is [`JsonValue::Str`].
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the element slice when this value is [`JsonValue::Array`].
    pub(crate) fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            Self::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    /// Returns the entry slice when this value is [`JsonValue::Object`].
    pub(crate) fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            Self::Object(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }

    /// Returns a mutable entry vector when this value is [`JsonValue::Object`].
    pub(crate) fn as_object_mut(&mut self) -> Option<&mut Vec<(String, JsonValue)>> {
        match self {
            Self::Object(entries) => Some(entries),
            _ => None,
        }
    }

    /// Looks up `key` inside [`JsonValue::Object`].
    pub(crate) fn get(&self, key: &str) -> Option<&JsonValue> {
        self.as_object()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// Sets `key` to `value` in `entries`, preserving position if already present.
    pub(crate) fn set_object_key(
        entries: &mut Vec<(String, JsonValue)>,
        key: &str,
        value: JsonValue,
    ) {
        if let Some((_, existing)) = entries.iter_mut().find(|(k, _)| k == key) {
            *existing = value;
        } else {
            entries.push((key.to_owned(), value));
        }
    }
}

/// Recursive-descent JSON cursor over Unicode scalar characters.
struct Parser {
    /// Input character buffer.
    chars: Vec<char>,
    /// Current read cursor index inside `chars`.
    pos: usize,
}

impl Parser {
    /// Creates a new cursor positioned at the start of `input`.
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    /// Skips ASCII JSON whitespace characters.
    fn skip_ws(&mut self) {
        while let Some(&c) = self.chars.get(self.pos) {
            if matches!(c, ' ' | '\n' | '\r' | '\t') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Returns the next unconsumed character without advancing `pos`.
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// Parses any valid JSON value starting at the current cursor.
    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_ws();
        let ch = self
            .peek()
            .ok_or_else(|| "Unexpected end of JSON input".to_string())?;
        match ch {
            '{' => self.parse_object(),
            '[' => self.parse_array(),
            '"' => self.parse_string().map(JsonValue::Str),
            't' => self.expect_literal("true", JsonValue::Bool(true)),
            'f' => self.expect_literal("false", JsonValue::Bool(false)),
            'n' => self.expect_literal("null", JsonValue::Null),
            '-' | '0'..='9' => self.parse_number(),
            other => Err(format!("Unexpected character `{other}` in JSON")),
        }
    }

    /// Consumes an exact keyword literal and returns `value`.
    fn expect_literal(&mut self, literal: &str, value: JsonValue) -> Result<JsonValue, String> {
        for expected in literal.chars() {
            let actual = self.chars.get(self.pos).copied();
            if actual != Some(expected) {
                return Err(format!("Expected `{literal}` in JSON"));
            }
            self.pos += 1;
        }
        Ok(value)
    }

    /// Parses one JSON object `{ ... }` while preserving key order.
    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.pos += 1;
        self.skip_ws();
        let mut entries = Vec::new();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(JsonValue::Object(entries));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some('"') {
                return Err("Expected string key in JSON object".to_string());
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(':') {
                return Err(format!("Expected `:` after object key `{key}`"));
            }
            self.pos += 1;
            let val = self.parse_value()?;
            entries.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some('}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(entries));
                }
                _ => return Err("Expected `,` or `}` in JSON object".to_string()),
            }
        }
    }

    /// Parses one JSON array `[ ... ]`.
    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.pos += 1;
        self.skip_ws();
        let mut items = Vec::new();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some(']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err("Expected `,` or `]` in JSON array".to_string()),
            }
        }
    }

    /// Parses a double-quoted JSON string including `\uXXXX` and surrogate escapes.
    fn parse_string(&mut self) -> Result<String, String> {
        if self.peek() != Some('"') {
            return Err("Expected opening `\"` for JSON string".to_string());
        }
        self.pos += 1;
        let mut out = String::new();
        while let Some(ch) = self.chars.get(self.pos).copied() {
            self.pos += 1;
            if ch == '"' {
                return Ok(out);
            }
            if ch != '\\' {
                out.push(ch);
                continue;
            }
            let esc = self
                .chars
                .get(self.pos)
                .copied()
                .ok_or_else(|| "Unterminated JSON string escape".to_string())?;
            self.pos += 1;
            match esc {
                '"' | '\\' | '/' => out.push(esc),
                'b' => out.push('\u{0008}'),
                'f' => out.push('\u{000C}'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => out.push(self.parse_unicode_escape()?),
                other => return Err(format!("Unsupported JSON escape `\\{other}`")),
            }
        }
        Err("Unterminated JSON string literal".to_string())
    }

    /// Parses one `\uXXXX` escape, consuming a trailing UTF-16 low surrogate when needed.
    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let high = self.read_hex_u16()?;
        if (0xD800..=0xDBFF).contains(&high) {
            if self.chars.get(self.pos) == Some(&'\\') && self.chars.get(self.pos + 1) == Some(&'u')
            {
                self.pos += 2;
                let low = self.read_hex_u16()?;
                if let Some(Ok(decoded)) = char::decode_utf16([high, low]).next() {
                    return Ok(decoded);
                }
            }
            return Err("Invalid UTF-16 surrogate pair in JSON string".to_string());
        }
        char::from_u32(u32::from(high))
            .ok_or_else(|| format!("Invalid Unicode code point U+{high:04X} in JSON"))
    }

    /// Reads 4 hexadecimal digits into a `u16`.
    fn read_hex_u16(&mut self) -> Result<u16, String> {
        let mut val: u16 = 0;
        for _ in 0..4 {
            let c = self
                .chars
                .get(self.pos)
                .copied()
                .ok_or_else(|| "Truncated `\\u` escape in JSON".to_string())?;
            self.pos += 1;
            let digit = c
                .to_digit(16)
                .ok_or_else(|| format!("Invalid hex digit `{c}` in `\\u` escape"))?;
            val = (val << 4) | (digit as u16);
        }
        Ok(val)
    }

    /// Consumes a JSON numeric token into [`JsonValue::Number`].
    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        let mut digits = 0usize;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                digits += 1;
                self.pos += 1;
            } else {
                break;
            }
        }
        if digits == 0 {
            return Err("Expected digit in JSON number".to_string());
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            let mut frac = 0usize;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    frac += 1;
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if frac == 0 {
                return Err("Expected fractional digits in JSON number".to_string());
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            let mut exp = 0usize;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    exp += 1;
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if exp == 0 {
                return Err("Expected exponent digits in JSON number".to_string());
            }
        }
        let raw: String = self.chars[start..self.pos].iter().collect();
        Ok(JsonValue::Number(raw))
    }
}

/// Parses a complete UTF-8 JSON document into a [`JsonValue`].
pub(crate) fn parse_json(input: &str) -> Result<JsonValue, String> {
    let mut parser = Parser::new(input);
    let val = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != parser.chars.len() {
        return Err("Trailing characters after JSON value".to_string());
    }
    Ok(val)
}

/// Formats `value` as 2-space pretty JSON terminated by a single LF newline.
pub(crate) fn format_json_pretty(value: &JsonValue) -> String {
    let mut out = String::new();
    write_pretty(&mut out, value, 0);
    out.push('\n');
    out
}

/// Writes formatted `value` into `out` at indentation `depth`.
fn write_pretty(out: &mut String, value: &JsonValue, depth: usize) {
    match value {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(true) => out.push_str("true"),
        JsonValue::Bool(false) => out.push_str("false"),
        JsonValue::Number(raw) => out.push_str(raw),
        JsonValue::Str(text) => write_escaped_str(out, text),
        JsonValue::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (idx, item) in items.iter().enumerate() {
                push_indent(out, depth + 1);
                write_pretty(out, item, depth + 1);
                if idx + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, depth);
            out.push(']');
        }
        JsonValue::Object(entries) => {
            if entries.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (idx, (key, val)) in entries.iter().enumerate() {
                push_indent(out, depth + 1);
                write_escaped_str(out, key);
                out.push_str(": ");
                write_pretty(out, val, depth + 1);
                if idx + 1 < entries.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, depth);
            out.push('}');
        }
    }
}

/// Appends `depth * 2` spaces to `out`.
fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Writes `text` as a quoted and escaped JSON string literal.
fn write_escaped_str(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
