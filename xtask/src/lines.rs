// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Non-comment, non-blank code line limit checks for Rust and TypeScript files.

use std::fs;
use std::path::Path;

use crate::lexer::{count_ts_code_lines, lex_rust};
use crate::workspace::{WorkspaceFiles, rel_display};

/// Maximum permitted non-comment, non-blank code lines in a single source file.
const MAX_CODE_LINES: usize = 800;

/// Checks code line counts across all `.rs` and `.ts` files against [`MAX_CODE_LINES`].
pub(crate) fn check_line_limits(root: &Path, files: &WorkspaceFiles) -> Vec<String> {
    let mut errors = Vec::new();

    for path in &files.rust_files {
        let Ok(src) = fs::read_to_string(path) else {
            continue;
        };
        let count = lex_rust(&src).code_lines;
        if count > MAX_CODE_LINES {
            errors.push(format!(
                "{}: {count} code lines exceed limit of {MAX_CODE_LINES}",
                rel_display(root, path)
            ));
        }
    }

    for path in &files.ts_files {
        let Ok(src) = fs::read_to_string(path) else {
            continue;
        };
        let count = count_ts_code_lines(&src);
        if count > MAX_CODE_LINES {
            errors.push(format!(
                "{}: {count} code lines exceed limit of {MAX_CODE_LINES}",
                rel_display(root, path)
            ));
        }
    }

    errors
}
