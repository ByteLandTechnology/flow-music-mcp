// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Local SQLite storage for the index of downloaded files.
//!
//! Stores records in `<home_dir>/flow-music-mcp.db` with `0600` file permissions on Unix.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params, types::Value as SqlValue};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::db_path;
use crate::private_fs::{create_private_dir_all, set_private_file_permissions};

/// MCP resource URI prefix for a download record selected by integer ID.
pub(crate) const DOWNLOAD_RESOURCE_PREFIX: &str = "music://downloads/";

/// MCP resource URI prefix for the latest download of a Google Flow Music clip or video ID.
pub(crate) const DOWNLOAD_ASSET_RESOURCE_PREFIX: &str = "music://downloads/asset/";

/// Builds the canonical MCP resource URI (`music://downloads/<id>`) for download `id`.
pub(crate) fn download_resource_uri(id: i64) -> String {
    format!("{}{}", DOWNLOAD_RESOURCE_PREFIX, id)
}

/// Returns a clean MIME type string, guessing from `file_path` when `mime_type` is missing or generic.
pub(crate) fn effective_mime_type(mime_type: Option<&str>, file_path: &str) -> String {
    if let Some(raw) = mime_type {
        let base = raw
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !base.is_empty() && base != "application/octet-stream" {
            return base;
        }
    }
    mime_guess::from_path(file_path)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

/// SQLite SELECT column list for [`DownloadRecord`] queries.
const SELECT_DOWNLOAD_SQL: &str = "SELECT id, asset_id, session_id, session_title, name, kind, prompt, source_url, file_path, file_name, mime_type, size_bytes, asset_created_at, downloaded_at FROM downloads";

/// Input fields for recording a downloaded file in SQLite.
#[derive(Debug, Clone)]
pub(crate) struct NewDownloadRecord {
    /// Google Flow Music clip or video ID.
    pub(crate) asset_id: String,
    /// Google Flow Music session (conversation) ID when known.
    pub(crate) session_id: Option<String>,
    /// Google Flow Music session title when known.
    pub(crate) session_title: Option<String>,
    /// Asset display name.
    pub(crate) name: String,
    /// File kind (`"audio"`, `"stem"`, `"video"`, or `"image"`).
    pub(crate) kind: String,
    /// Generation prompt when available.
    pub(crate) prompt: Option<String>,
    /// Resolved download URL when available.
    pub(crate) source_url: Option<String>,
    /// Resolved local path on disk.
    pub(crate) file_path: String,
    /// Leaf file name on disk.
    pub(crate) file_name: String,
    /// Validated HTTP `Content-Type` or sniffed MIME type when available.
    pub(crate) mime_type: Option<String>,
    /// Downloaded file size in bytes.
    pub(crate) size_bytes: u64,
    /// Google Flow Music clip creation timestamp string when available.
    pub(crate) asset_created_at: Option<String>,
}

/// Indexed download record returned from SQLite queries and downloads.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct DownloadRecord {
    /// Row ID in the `downloads` table.
    pub(crate) id: i64,
    /// Google Flow Music clip or video ID.
    pub(crate) asset_id: String,
    /// Google Flow Music session (conversation) ID when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_id: Option<String>,
    /// Google Flow Music session title when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_title: Option<String>,
    /// Asset display name.
    pub(crate) name: String,
    /// File kind (`"audio"`, `"stem"`, `"video"`, or `"image"`).
    pub(crate) kind: String,
    /// Generation prompt when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prompt: Option<String>,
    /// Source media URL when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_url: Option<String>,
    /// Absolute or resolved local file path.
    pub(crate) file_path: String,
    /// Leaf file name on disk.
    pub(crate) file_name: String,
    /// HTTP `Content-Type` when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mime_type: Option<String>,
    /// Saved file size in bytes.
    pub(crate) size_bytes: u64,
    /// Google Flow Music clip `created_at` string when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) asset_created_at: Option<String>,
    /// Download timestamp in Unix epoch milliseconds.
    pub(crate) downloaded_at: i64,
    /// True when `file_path` currently exists on disk.
    pub(crate) exists: bool,
    /// MCP resource URI (`music://downloads/<id>`) for reading the file over MCP.
    pub(crate) resource_uri: String,
}

/// Parameters for `music_downloads`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub(crate) struct DownloadsParams {
    /// Action name: "list" (default), "get", "read_chunk", "delete", "prune", or "clear".
    pub(crate) action: Option<String>,
    /// Download record row ID for "get", "read_chunk", or "delete".
    #[schemars(range(min = 1))]
    pub(crate) id: Option<i64>,
    /// Google Flow Music clip or video ID for "get", "read_chunk", or "delete".
    pub(crate) asset_id: Option<String>,
    /// Case-insensitive substring that matches asset ID, name, prompt, file name, or path for "list".
    pub(crate) query: Option<String>,
    /// File kind filter for "list": "audio", "stem", "video", "image", or "all" (default).
    pub(crate) kind: Option<String>,
    /// Filter by Google Flow Music session ID for "list".
    pub(crate) session_id: Option<String>,
    /// Return only rows whose file currently exists on disk for "list" (default false).
    pub(crate) existing_only: Option<bool>,
    /// Maximum number of download records to return for "list" (1..=500, default 50).
    #[schemars(range(min = 1, max = 500))]
    pub(crate) limit: Option<usize>,
    /// Byte offset in the downloaded file for "read_chunk" (default 0).
    pub(crate) offset: Option<u64>,
    /// Maximum chunk size in bytes for "read_chunk" (1..=4_194_304, default 1_048_576).
    #[schemars(range(min = 1, max = 4_194_304))]
    pub(crate) length: Option<usize>,
    /// Delete only files downloaded more than this many days ago when action is "clear".
    pub(crate) older_than_days: Option<u64>,
}

/// SQLite DDL creating the `downloads` table and its indexes.
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS downloads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id TEXT NOT NULL,
    session_id TEXT,
    session_title TEXT,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    prompt TEXT,
    source_url TEXT,
    file_path TEXT NOT NULL UNIQUE,
    file_name TEXT NOT NULL,
    mime_type TEXT,
    size_bytes INTEGER NOT NULL,
    asset_created_at TEXT,
    downloaded_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_downloads_asset_id ON downloads(asset_id);
CREATE INDEX IF NOT EXISTS idx_downloads_downloaded_at ON downloads(downloaded_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_downloads_session_id ON downloads(session_id);
"#;

/// Returns the current Unix epoch time in milliseconds.
pub(crate) fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

/// Maps one row selected by [`SELECT_DOWNLOAD_SQL`] into a [`DownloadRecord`].
fn row_to_download_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<DownloadRecord> {
    let id: i64 = row.get(0)?;
    let file_path: String = row.get(8)?;
    let size_raw: i64 = row.get(11)?;
    let exists = Path::new(&file_path).is_file();
    Ok(DownloadRecord {
        id,
        asset_id: row.get(1)?,
        session_id: row.get(2)?,
        session_title: row.get(3)?,
        name: row.get(4)?,
        kind: row.get(5)?,
        prompt: row.get(6)?,
        source_url: row.get(7)?,
        file_path,
        file_name: row.get(9)?,
        mime_type: row.get(10)?,
        size_bytes: size_raw.max(0) as u64,
        asset_created_at: row.get(12)?,
        downloaded_at: row.get(13)?,
        exists,
        resource_uri: download_resource_uri(id),
    })
}

/// Queries at most one [`DownloadRecord`] using [`SELECT_DOWNLOAD_SQL`] and `suffix_sql`.
fn query_download_opt<P: rusqlite::Params>(
    conn: &Connection,
    suffix_sql: &str,
    params: P,
) -> rusqlite::Result<Option<DownloadRecord>> {
    let sql = format!("{} {}", SELECT_DOWNLOAD_SQL, suffix_sql);
    conn.query_row(&sql, params, row_to_download_record)
        .optional()
}

/// Queries zero or more [`DownloadRecord`] rows using [`SELECT_DOWNLOAD_SQL`] and `suffix_sql`.
fn query_download_vec<P: rusqlite::Params>(
    conn: &Connection,
    suffix_sql: &str,
    params: P,
) -> rusqlite::Result<Vec<DownloadRecord>> {
    let sql = format!("{} {}", SELECT_DOWNLOAD_SQL, suffix_sql);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params, row_to_download_record)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Deletes `ids` from `downloads` in one transaction and returns `(deleted_rows, remaining_rows)`.
fn delete_download_rows_tx(conn: &mut Connection, ids: &[i64]) -> Result<(usize, usize)> {
    let tx = conn.transaction()?;
    let mut deleted = 0usize;
    {
        let mut stmt = tx.prepare("DELETE FROM downloads WHERE id = ?1")?;
        for id in ids {
            deleted += stmt.execute(params![id])?;
        }
    }
    tx.commit()?;
    let remaining: i64 = conn.query_row("SELECT COUNT(*) FROM downloads", [], |row| row.get(0))?;
    Ok((deleted, remaining.max(0) as usize))
}

/// Creates the SQLite database file and parent directory with owner-only permissions.
fn ensure_private_db_file(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    create_private_dir_all(parent)?;

    let mut opts = OpenOptions::new();
    opts.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
        .with_context(|| format!("Failed to open SQLite database at {}", path.display()))?;
    set_private_file_permissions(path);
    Ok(())
}

/// Opens and initializes `<home_dir>/flow-music-mcp.db` with WAL pragmas and schema tables.
fn open_initialized_connection() -> Result<Connection> {
    let path = db_path();
    ensure_private_db_file(&path)?;

    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to connect to SQLite database at {}", path.display()))?;
    set_private_file_permissions(&path);
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;
         PRAGMA foreign_keys = ON;",
    )?;
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(conn)
}

/// Thread-safe handle to `<home_dir>/flow-music-mcp.db`.
#[derive(Clone, Default)]
pub(crate) struct MusicDb {
    /// Lazily opened SQLite connection guarded by a mutex.
    conn: Arc<Mutex<Option<Connection>>>,
}

impl MusicDb {
    /// Creates a lazy SQLite database handle backed by `<home_dir>/flow-music-mcp.db`.
    pub(crate) fn new() -> Self {
        Self {
            conn: Arc::new(Mutex::new(None)),
        }
    }

    /// Acquires the SQLite connection lock, opens the database if needed, and runs `op`.
    fn with_conn<F, R>(&self, op: F) -> Result<R>
    where
        F: FnOnce(&mut Connection) -> Result<R>,
    {
        let mut guard = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("SQLite database mutex poisoned"))?;
        let conn = match guard.as_mut() {
            Some(c) => c,
            None => guard.insert(open_initialized_connection()?),
        };
        op(conn)
    }

    /// Executes `op` on a blocking thread with exclusive access to the SQLite connection.
    async fn run_blocking<F, R>(&self, op: F) -> Result<R>
    where
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.with_conn(op))
            .await
            .map_err(|e| anyhow::anyhow!("SQLite worker task failed: {}", e))?
    }

    /// Returns the total number of indexed download rows.
    pub(crate) async fn count_downloads(&self) -> Result<usize> {
        self.run_blocking(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM downloads", [], |row| row.get(0))?;
            Ok(count.max(0) as usize)
        })
        .await
    }

    /// Inserts or updates a downloaded file record and returns the persisted row.
    pub(crate) async fn record_download(&self, rec: NewDownloadRecord) -> Result<DownloadRecord> {
        let downloaded_at = now_epoch_ms();
        let size_bytes = rec.size_bytes.min(i64::MAX as u64) as i64;
        self.run_blocking(move |conn| {
            conn.execute(
                "INSERT INTO downloads (
                    asset_id, session_id, session_title, name, kind, prompt,
                    source_url, file_path, file_name, mime_type, size_bytes,
                    asset_created_at, downloaded_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(file_path) DO UPDATE SET
                    asset_id = excluded.asset_id,
                    session_id = excluded.session_id,
                    session_title = excluded.session_title,
                    name = excluded.name,
                    kind = excluded.kind,
                    prompt = excluded.prompt,
                    source_url = excluded.source_url,
                    file_name = excluded.file_name,
                    mime_type = excluded.mime_type,
                    size_bytes = excluded.size_bytes,
                    asset_created_at = excluded.asset_created_at,
                    downloaded_at = excluded.downloaded_at",
                params![
                    rec.asset_id,
                    rec.session_id,
                    rec.session_title,
                    rec.name,
                    rec.kind,
                    rec.prompt,
                    rec.source_url,
                    rec.file_path,
                    rec.file_name,
                    rec.mime_type,
                    size_bytes,
                    rec.asset_created_at,
                    downloaded_at,
                ],
            )?;
            let row = query_download_opt(conn, "WHERE file_path = ?1", params![rec.file_path])?
                .ok_or_else(|| anyhow::anyhow!("Inserted download row missing after upsert"))?;
            Ok(row)
        })
        .await
    }

    /// Queries the `downloads` table with optional filters and returns `(total_indexed, total_bytes, records)`.
    pub(crate) async fn list_downloads(
        &self,
        params: DownloadsParams,
    ) -> Result<(usize, u64, Vec<DownloadRecord>)> {
        self.run_blocking(move |conn| {
            let (total, total_bytes): (i64, i64) = conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM downloads",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;

            let mut clauses: Vec<&'static str> = Vec::new();
            let mut bind_vals: Vec<SqlValue> = Vec::new();

            if let Some(kind) = params
                .kind
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty() && !k.eq_ignore_ascii_case("all"))
            {
                clauses.push("LOWER(kind) = LOWER(?)");
                bind_vals.push(SqlValue::Text(kind.to_string()));
            }

            if let Some(session_id) = params
                .session_id
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
            {
                clauses.push("session_id = ?");
                bind_vals.push(SqlValue::Text(session_id.to_string()));
            }

            if let Some(query) = params
                .query
                .as_deref()
                .map(str::trim)
                .filter(|q| !q.is_empty())
            {
                let needle = format!("%{}%", query.to_lowercase());
                clauses.push(
                    "(LOWER(asset_id) LIKE ? OR LOWER(name) LIKE ? OR LOWER(COALESCE(prompt, '')) LIKE ? OR LOWER(file_name) LIKE ? OR LOWER(file_path) LIKE ?)",
                );
                for _ in 0..5 {
                    bind_vals.push(SqlValue::Text(needle.clone()));
                }
            }

            let mut sql = String::from(SELECT_DOWNLOAD_SQL);
            if !clauses.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&clauses.join(" AND "));
            }
            sql.push_str(" ORDER BY downloaded_at DESC, id DESC");

            let existing_only = params.existing_only.unwrap_or(false);
            let limit = params.limit.unwrap_or(50).clamp(1, 500);
            if !existing_only {
                sql.push_str(" LIMIT ?");
                bind_vals.push(SqlValue::Integer(limit as i64));
            }

            let mut stmt = conn.prepare(&sql)?;
            let rows =
                stmt.query_map(rusqlite::params_from_iter(bind_vals), row_to_download_record)?;
            let mut records = Vec::new();
            for row in rows {
                let rec = row?;
                if existing_only && !rec.exists {
                    continue;
                }
                records.push(rec);
                if records.len() >= limit {
                    break;
                }
            }

            Ok((total.max(0) as usize, total_bytes.max(0) as u64, records))
        })
        .await
    }

    /// Returns a download record by row ID if found.
    pub(crate) async fn get_download_by_id(&self, id: i64) -> Result<Option<DownloadRecord>> {
        self.run_blocking(move |conn| Ok(query_download_opt(conn, "WHERE id = ?1", params![id])?))
            .await
    }

    /// Returns the most recent download record for `asset_id` regardless of disk presence.
    pub(crate) async fn get_latest_download_for_asset(
        &self,
        asset_id: &str,
    ) -> Result<Option<DownloadRecord>> {
        let asset_id = asset_id.to_string();
        self.run_blocking(move |conn| {
            Ok(query_download_opt(
                conn,
                "WHERE asset_id = ?1 ORDER BY downloaded_at DESC, id DESC LIMIT 1",
                params![asset_id],
            )?)
        })
        .await
    }

    /// Returns all indexed download records whose files currently exist on disk.
    pub(crate) async fn list_all_existing_downloads(&self) -> Result<Vec<DownloadRecord>> {
        self.run_blocking(|conn| {
            let mut out = query_download_vec(conn, "ORDER BY downloaded_at DESC, id DESC", [])?;
            out.retain(|rec| rec.exists);
            Ok(out)
        })
        .await
    }

    /// Selects download records by row `id` or `asset_id`.
    pub(crate) async fn select_downloads_by_selector(
        &self,
        id: Option<i64>,
        asset_id: Option<&str>,
    ) -> Result<Vec<DownloadRecord>> {
        let asset_id = asset_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if id.is_none() && asset_id.is_none() {
            anyhow::bail!("Specify 'id' or 'asset_id'");
        }
        self.run_blocking(move |conn| {
            if let Some(row_id) = id {
                Ok(query_download_opt(conn, "WHERE id = ?1", params![row_id])?
                    .into_iter()
                    .collect())
            } else if let Some(aid) = asset_id {
                Ok(query_download_vec(
                    conn,
                    "WHERE asset_id = ?1 ORDER BY downloaded_at DESC, id DESC",
                    params![aid],
                )?)
            } else {
                anyhow::bail!("Specify 'id' or 'asset_id'")
            }
        })
        .await
    }

    /// Selects download records targeted by a clear operation.
    pub(crate) async fn select_downloads_for_clear(
        &self,
        older_than_days: Option<u64>,
    ) -> Result<Vec<DownloadRecord>> {
        let cutoff = older_than_days.map(|days| {
            let ms = (days as i128)
                .saturating_mul(86_400_000)
                .min(i64::MAX as i128) as i64;
            now_epoch_ms().saturating_sub(ms)
        });
        self.run_blocking(move |conn| {
            if let Some(cutoff_ms) = cutoff {
                Ok(query_download_vec(
                    conn,
                    "WHERE downloaded_at < ?1 ORDER BY downloaded_at DESC, id DESC",
                    params![cutoff_ms],
                )?)
            } else {
                Ok(query_download_vec(
                    conn,
                    "ORDER BY downloaded_at DESC, id DESC",
                    [],
                )?)
            }
        })
        .await
    }

    /// Deletes download rows for `ids` in one transaction and returns `(deleted_rows, remaining_rows)`.
    pub(crate) async fn delete_downloads_by_ids(&self, ids: &[i64]) -> Result<(usize, usize)> {
        let ids = ids.to_vec();
        self.run_blocking(move |conn| delete_download_rows_tx(conn, &ids))
            .await
    }

    /// Deletes SQLite rows whose indexed files no longer exist on disk and returns `(pruned_rows, remaining_rows)`.
    pub(crate) async fn prune_missing_downloads(&self) -> Result<(usize, usize)> {
        self.run_blocking(|conn| {
            let missing_ids: Vec<i64> = {
                let mut stmt = conn.prepare("SELECT id, file_path FROM downloads")?;
                let rows = stmt.query_map([], |row| {
                    let id: i64 = row.get(0)?;
                    let path: String = row.get(1)?;
                    Ok((id, path))
                })?;
                let mut ids = Vec::new();
                for row in rows {
                    let (id, path) = row?;
                    if !Path::new(&path).is_file() {
                        ids.push(id);
                    }
                }
                ids
            };
            delete_download_rows_tx(conn, &missing_ids)
        })
        .await
    }

    /// Returns the `asset_id` currently indexed at `file_path`, if any.
    pub(crate) async fn find_owner_for_path(&self, file_path: &str) -> Result<Option<String>> {
        let file_path = file_path.to_string();
        self.run_blocking(move |conn| {
            let owner = conn
                .query_row(
                    "SELECT asset_id FROM downloads WHERE file_path = ?1",
                    params![file_path],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(owner)
        })
        .await
    }

    /// Returns the most recent download record for `asset_id` whose file still exists on disk.
    pub(crate) async fn find_existing_file_for_asset(
        &self,
        asset_id: &str,
    ) -> Result<Option<DownloadRecord>> {
        let asset_id = asset_id.to_string();
        self.run_blocking(move |conn| {
            let rows = query_download_vec(
                conn,
                "WHERE asset_id = ?1 ORDER BY downloaded_at DESC, id DESC",
                params![asset_id],
            )?;
            Ok(rows.into_iter().find(|rec| rec.exists))
        })
        .await
    }
}
