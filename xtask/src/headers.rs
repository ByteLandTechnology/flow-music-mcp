// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Two-line SPDX license header and LF line ending verification.

use std::fs;
use std::path::Path;

use crate::workspace::{WorkspaceFiles, rel_display};

/// Required first line for `.rs` and `.ts` source files.
const COPYRIGHT_LINE: &str = "// Copyright 2026 ByteLand Technology Limited";

/// Required second line for `.rs` and `.ts` source files.
const SPDX_LINE: &str = "// SPDX-License-Identifier: MIT";

/// Required shebang line on the npm CLI launcher script.
const SHEBANG_LINE: &str = "#!/usr/bin/env node";

/// Verifies exact two-line SPDX headers and LF line endings across workspace files.
pub(crate) fn check_license_headers(root: &Path, files: &WorkspaceFiles) -> Vec<String> {
    let mut errors = Vec::new();

    for path in &files.text_files {
        match fs::read_to_string(path) {
            Ok(content) => {
                if content.contains('\r') {
                    errors.push(format!(
                        "{}: contains CR/CRLF line endings; use LF only",
                        rel_display(root, path)
                    ));
                }
            }
            Err(err) => {
                errors.push(format!(
                    "{}: failed to read UTF-8 text: {err}",
                    rel_display(root, path)
                ));
            }
        }
    }

    for path in files.rust_files.iter().chain(files.ts_files.iter()) {
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let mut lines = content.lines();
        let first = lines.next().unwrap_or("");
        let second = lines.next().unwrap_or("");
        if first != COPYRIGHT_LINE || second != SPDX_LINE {
            errors.push(format!(
                "{}: missing exact two-line copyright and SPDX header",
                rel_display(root, path)
            ));
        }
    }

    for path in &files.launcher_js_files {
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let mut lines = content.lines();
        let first = lines.next().unwrap_or("");
        let second = lines.next().unwrap_or("");
        let third = lines.next().unwrap_or("");
        if first != SHEBANG_LINE || second != COPYRIGHT_LINE || third != SPDX_LINE {
            errors.push(format!(
                "{}: missing shebang and exact two-line copyright and SPDX header",
                rel_display(root, path)
            ));
        }
    }

    errors
}
