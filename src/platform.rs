// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Platform-specific file permissions and process control.
//!
//! Each item in this module exists on every platform with the same signature.
//! Platform `cfg` attributes stay inside function bodies, so that no item is unused on one platform.

use std::ffi::OsStr;
use std::fs::File;
use std::path::Path;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};

/// Process ID and parent process ID of a process that matches a command-line argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MatchedProcess {
    /// Process ID.
    pub(crate) pid: u32,
    /// Parent process ID, if the operating system reports one.
    pub(crate) parent: Option<u32>,
}

/// Creates `dir` and missing parents with owner-only (`0o700`) permissions on Unix.
pub(crate) fn create_private_dir_all(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Sets owner read/write permissions (`0o600`) on `path` on Unix systems.
pub(crate) fn set_private_file_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Opens `path` for reading and writing, and creates it with `0o600` permissions on Unix if it does not exist.
pub(crate) fn open_private_file(path: &Path) -> std::io::Result<File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts.open(path)?;
    set_private_file_permissions(path);
    Ok(file)
}

/// Returns the Unix permission bits of `path`, or `None` when `path` is missing or the platform has no such bits.
pub(crate) fn file_mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .ok()
            .map(|meta| meta.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Returns a process table with basic information for `pid` only.
fn system_with_process(pid: u32) -> System {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing(),
    );
    sys
}

/// Returns `true` when process `pid` exists.
pub(crate) fn process_exists(pid: u32) -> bool {
    system_with_process(pid)
        .process(Pid::from_u32(pid))
        .is_some()
}

/// Asks process `pid` to stop normally, and returns `true` when the request was sent.
///
/// Unix sends `SIGINT`. Windows has no `SIGINT` for other processes, thus it asks the
/// windows of the process to close with `taskkill` without `/F`.
pub(crate) fn interrupt_process(pid: u32) -> bool {
    let sys = system_with_process(pid);
    let Some(process) = sys.process(Pid::from_u32(pid)) else {
        return false;
    };
    match process.kill_with(Signal::Interrupt) {
        Some(sent) => sent,
        None => request_window_close(pid),
    }
}

/// Asks the windows of process `pid` to close (Windows only); returns `false` on other platforms.
fn request_window_close(pid: u32) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("taskkill")
            .arg("/PID")
            .arg(pid.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

/// Stops process `pid` immediately, and returns `true` when the request was sent.
pub(crate) fn kill_process(pid: u32) -> bool {
    system_with_process(pid)
        .process(Pid::from_u32(pid))
        .is_some_and(|process| process.kill())
}

/// Returns the top-level processes that have `arg` as one complete command-line argument.
///
/// A matching process whose parent also matches (for example a Chrome helper process) is not returned.
pub(crate) fn top_level_processes_with_arg(arg: &OsStr) -> Vec<MatchedProcess> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    let matched: Vec<MatchedProcess> = sys
        .processes()
        .values()
        .filter(|process| process.cmd().iter().any(|a| a.as_os_str() == arg))
        .map(|process| MatchedProcess {
            pid: process.pid().as_u32(),
            parent: process.parent().map(Pid::as_u32),
        })
        .collect();
    matched
        .iter()
        .filter(|m| {
            m.parent
                .is_none_or(|parent| !matched.iter().any(|other| other.pid == parent))
        })
        .copied()
        .collect()
}

/// Activates the windows of process `pid` so that the user sees them (macOS only).
pub(crate) fn focus_process(pid: u32) {
    #[cfg(target_os = "macos")]
    {
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let script = format!(
                "ObjC.import('AppKit'); const a = $.NSRunningApplication.runningApplicationWithProcessIdentifier({pid}); if (!a.isNil()) a.activateWithOptions(3);"
            );
            let _ = tokio::process::Command::new("/usr/bin/osascript")
                .arg("-l")
                .arg("JavaScript")
                .arg("-e")
                .arg(script)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = pid;
}
