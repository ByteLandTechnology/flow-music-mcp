// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Application service that routes tools, runs queued jobs, and serves downloads.

use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rmcp::model::{Resource, ResourceContents};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::auth::{self, LoginParams};
use crate::chat::{self, ChatParams, VideoParams};
use crate::client::MusicClient;
use crate::collections::{self, PlaylistsParams, SpacesParams};
use crate::config;
use crate::db::{
    DOWNLOAD_ASSET_RESOURCE_PREFIX, DOWNLOAD_RESOURCE_PREFIX, DownloadRecord, DownloadsParams,
    MusicDb, effective_mime_type,
};
use crate::download::{self, DownloadSource, DownloadTarget};
use crate::edits::{self, EditParams, StemsParams};
use crate::engine::{ChromeEngine, ENGINE_NAME};
use crate::generation::{self, GenerateParams, LyricsParams};
use crate::library::{self, SessionsParams, SongsParams};
use crate::media_files::{self, MediaKind};
use crate::producer::fetch_clips;
use crate::queue::{JobKind, JobQueue, JobRunner, JobView, MusicJob, WaitParams};
use crate::uploads::{self, UploadParams};

/// Maximum downloaded file size served directly through MCP `resources/read` (`64 MiB`).
const MAX_RESOURCE_READ_BYTES: u64 = 64 * 1024 * 1024;

/// Parameters for `music_status`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct StatusParams {
    /// Start Chrome if needed and read the account, plan, and credits (default false).
    pub(crate) probe: Option<bool>,
}

/// Parameters for `music_download`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct DownloadParams {
    /// Clip ID for "audio" and "image", or a clip ID or music video ID for "video".
    pub(crate) id: String,
    /// What to download: "audio" (default), "video", or "image" (the cover art).
    pub(crate) kind: Option<String>,
    /// Audio format for "audio": "m4a" (default), "wav", or "mp3".
    pub(crate) format: Option<String>,
}

/// Failure reasons when reading a downloaded file as an MCP resource.
pub(crate) enum ResourceReadError {
    /// The URI does not match a valid `music://downloads/...` format.
    InvalidUri(String),
    /// No matching database row or managed disk file exists.
    NotFound(String),
    /// The file exceeds the 64 MiB `resources/read` limit and must be fetched with `read_chunk`.
    TooLarge(String),
    /// Unexpected storage or I/O error.
    Internal(String),
}

/// Converts a [`DownloadRecord`] into an MCP [`Resource`] descriptor.
pub(crate) fn record_to_mcp_resource(rec: &DownloadRecord) -> Resource {
    let mime = effective_mime_type(rec.mime_type.as_deref(), &rec.file_path);
    let mut resource = Resource::new(rec.resource_uri.clone(), rec.file_name.clone())
        .with_mime_type(mime)
        .with_size(rec.size_bytes)
        .with_description(format!("Google Flow Music {} ({})", rec.kind, rec.asset_id));
    if !rec.name.is_empty() {
        resource = resource.with_title(rec.name.clone());
    }
    resource
}

/// Decodes the parameters of a queued job.
fn job_params<T: serde::de::DeserializeOwned>(job: &MusicJob) -> Result<T> {
    Ok(serde_json::from_value(job.params.clone())?)
}

/// Returns a short summary text of at most 120 characters.
fn summary(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 120 {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(119).collect();
    out.push('…');
    out
}

/// Returns a non-empty string at JSON `pointer` in `value`.
fn str_at(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Application service that coordinates tools, background jobs, and SQLite storage.
pub(crate) struct MusicService {
    /// Internal API client.
    client: MusicClient,
    /// Single-worker serial job queue.
    queue: Arc<JobQueue>,
    /// SQLite handle for the download index.
    db: MusicDb,
}

impl MusicService {
    /// Creates a [`MusicService`] connected to [`ChromeEngine`], [`MusicDb`], and a serial [`JobQueue`].
    pub(crate) fn new(engine: Arc<ChromeEngine>) -> Arc<Self> {
        let db = MusicDb::new();
        let client = MusicClient::new(engine);
        let runner_client = client.clone();
        let runner_db = db.clone();
        let runner: JobRunner = Arc::new(move |job, progress| {
            let client = runner_client.clone();
            let db = runner_db.clone();
            Box::pin(async move {
                match job.kind {
                    JobKind::Generate => {
                        generation::run_generate(&client, &db, job_params(&job)?, progress).await
                    }
                    JobKind::Lyrics => {
                        generation::run_lyrics(&client, job_params(&job)?, progress).await
                    }
                    JobKind::Edit => {
                        edits::run_edit(&client, &db, job_params(&job)?, progress).await
                    }
                    JobKind::Stems => {
                        edits::run_stems(&client, &db, job_params(&job)?, progress).await
                    }
                    JobKind::Chat => {
                        chat::run_chat_job(&client, &db, job_params(&job)?, progress).await
                    }
                    JobKind::Video => {
                        chat::run_video_job(&client, job_params(&job)?, progress).await
                    }
                }
            })
        });
        let queue = JobQueue::new(runner);
        Arc::new(Self { client, queue, db })
    }

    /// Returns session, account, job, and storage state.
    pub(crate) async fn status(&self, probe: bool) -> Result<Value> {
        let engine = self.client.engine();
        let (jobs, downloads) = tokio::join!(self.queue.list(), self.db.count_downloads());
        let mut account = Value::Null;
        if probe {
            let auth_state = auth::auth_state(engine).await;
            let signed_in = auth_state
                .as_ref()
                .ok()
                .and_then(|s| s.get("signed_in").and_then(Value::as_bool))
                == Some(true);
            account = json!({ "auth": auth_state.as_ref().ok() });
            if let Err(e) = &auth_state {
                account["error"] = json!(format!("{e:#}"));
            }
            if signed_in {
                let (credits, plan) = tokio::join!(
                    self.client.get("/billing/credits"),
                    self.client.get("/billing/subscription"),
                );
                account["credits"] = credits
                    .ok()
                    .and_then(|c| c.get("data").cloned())
                    .unwrap_or(Value::Null);
                account["plan"] = plan
                    .ok()
                    .and_then(|p| p.get("data").cloned())
                    .unwrap_or(Value::Null);
            }
        }
        Ok(json!({
            "engine": ENGINE_NAME,
            "page_ready": engine.has_page().await,
            "window_visible": engine.is_window_visible(),
            "url": engine.current_url().await.unwrap_or_default(),
            "account": account,
            "jobs": jobs,
            "downloads_indexed": downloads?,
            "db_path": config::db_path().to_string_lossy(),
            "downloads_dir": config::downloads_dir().to_string_lossy(),
            "profile_dir": config::chrome_profile_dir().to_string_lossy(),
            "cdp_endpoint": engine.cdp_endpoint().await,
            "chrome_binary": crate::chrome::find_chrome().map(|p| p.to_string_lossy().into_owned()),
        }))
    }

    /// Returns all currently tracked queue jobs.
    pub(crate) async fn list_jobs(&self) -> Vec<JobView> {
        self.queue.list().await
    }

    /// Adds a job of `kind` with `params` to the queue.
    async fn enqueue<T: Serialize>(
        &self,
        kind: JobKind,
        text: &str,
        params: &T,
    ) -> Result<JobView> {
        Ok(self
            .queue
            .enqueue(MusicJob {
                kind,
                summary: summary(text),
                params: serde_json::to_value(params)?,
            })
            .await)
    }

    /// Queues a song generation job.
    pub(crate) async fn generate(&self, req: GenerateParams) -> Result<JobView> {
        anyhow::ensure!(
            !req.prompt.trim().is_empty()
                || req.lyrics.as_deref().is_some_and(|l| !l.trim().is_empty()),
            "Give a sound prompt or lyrics"
        );
        self.enqueue(JobKind::Generate, &req.prompt, &req).await
    }

    /// Queues a lyrics job.
    pub(crate) async fn lyrics(&self, req: LyricsParams) -> Result<JobView> {
        anyhow::ensure!(!req.prompt.trim().is_empty(), "Give a lyrics prompt");
        self.enqueue(JobKind::Lyrics, &req.prompt, &req).await
    }

    /// Queues an extend, replace, or cover job.
    pub(crate) async fn edit(&self, req: EditParams) -> Result<JobView> {
        anyhow::ensure!(
            matches!(req.action.as_str(), "extend" | "replace" | "cover"),
            "Unknown edit action '{}'. Use extend, replace, or cover.",
            req.action
        );
        let text = format!("{} {}", req.action, req.clip_id);
        self.enqueue(JobKind::Edit, &text, &req).await
    }

    /// Queues a stem separation job.
    pub(crate) async fn stems(&self, req: StemsParams) -> Result<JobView> {
        let text = format!("stems {}", req.clip_id);
        self.enqueue(JobKind::Stems, &text, &req).await
    }

    /// Queues a chat message to the Producer.
    pub(crate) async fn chat(&self, req: ChatParams) -> Result<JobView> {
        anyhow::ensure!(!req.message.trim().is_empty(), "Give a message");
        self.enqueue(JobKind::Chat, &req.message, &req).await
    }

    /// Runs a video library action, or queues a video creation job.
    pub(crate) async fn video(&self, req: VideoParams) -> Result<Value> {
        if req.action.as_deref() == Some("create") {
            let text = format!("video {}", req.clip_id.as_deref().unwrap_or(""));
            let job = self.enqueue(JobKind::Video, &text, &req).await?;
            return Ok(json!({ "job": job }));
        }
        chat::video_library(&self.client, &req).await
    }

    /// Waits for `req.ids` or all jobs up to `req.timeout` seconds (default `600`).
    pub(crate) async fn wait(&self, req: WaitParams) -> Result<Value> {
        let ids = req
            .ids
            .filter(|v| !v.is_empty())
            .or_else(|| req.id.map(|s| vec![s]));
        let timeout_s = req.timeout.unwrap_or(600);
        let jobs = self
            .queue
            .wait_for(ids.as_deref(), timeout_s.saturating_mul(1000))
            .await?;
        Ok(json!({ "jobs": jobs }))
    }

    /// Cancels a queued or running job and stops its server-side session job.
    pub(crate) async fn cancel(&self, id: &str) -> Result<Value> {
        let job = self.queue.cancel(id).await?;
        let mut stopped = Value::Null;
        if let Some(conv) = job.conversation_id.as_deref() {
            stopped = match self
                .client
                .post(&format!("/conversations/{conv}/stop"), json!({}))
                .await
            {
                Ok(_) => json!(true),
                Err(e) => json!(format!("{e:#}")),
            };
        }
        Ok(json!({ "job": job, "server_stop": stopped }))
    }

    /// Retries a failed or cancelled job by `id`.
    pub(crate) async fn retry(&self, id: &str) -> Result<Value> {
        let res = self.queue.retry(id).await?;
        Ok(json!({ "job": res }))
    }

    /// Handles `music_login` actions.
    pub(crate) async fn login(&self, req: LoginParams) -> Result<Value> {
        auth::handle_login(self.client.engine(), req).await
    }

    /// Handles `music_songs` actions.
    pub(crate) async fn songs(&self, req: SongsParams) -> Result<Value> {
        library::songs(&self.client, req).await
    }

    /// Handles `music_sessions` actions.
    pub(crate) async fn sessions(&self, req: SessionsParams) -> Result<Value> {
        library::sessions(&self.client, req).await
    }

    /// Handles `music_playlists` actions.
    pub(crate) async fn playlists(&self, req: PlaylistsParams) -> Result<Value> {
        collections::playlists(&self.client, req).await
    }

    /// Handles `music_spaces` actions.
    pub(crate) async fn spaces(&self, req: SpacesParams) -> Result<Value> {
        collections::spaces(&self.client, req).await
    }

    /// Uploads a local audio or image file.
    pub(crate) async fn upload(&self, req: UploadParams) -> Result<Value> {
        uploads::upload(&self.client, req).await
    }

    /// Builds the download target of a music video record with ID `id`.
    async fn music_video_target(&self, id: &str) -> Result<DownloadTarget> {
        let res = self.client.get(&format!("/music-video/get/{id}")).await?;
        let video = res.get("data").unwrap_or(&res);
        let url = ["/video_url", "/url", "/final_video_url", "/output_url"]
            .iter()
            .find_map(|p| str_at(video, p))
            .ok_or_else(|| anyhow::anyhow!("Music video {} has no video file yet", id))?;
        Ok(DownloadTarget {
            id: id.to_string(),
            name: str_at(video, "/title").unwrap_or_else(|| id.to_string()),
            kind: "video".to_string(),
            media: MediaKind::Video,
            prompt: None,
            session_id: str_at(video, "/conversation_id"),
            session_title: None,
            created_at: str_at(video, "/created_at"),
            source: DownloadSource::PublicUrl(url),
        })
    }

    /// Downloads the audio, video, or cover art of a clip, or a music video, and indexes it.
    pub(crate) async fn download(&self, req: DownloadParams) -> Result<DownloadRecord> {
        let kind = req.kind.as_deref().unwrap_or("audio");
        let clip = fetch_clips(&self.client, std::slice::from_ref(&req.id))
            .await?
            .into_iter()
            .next();
        let target = match (kind, clip) {
            ("audio", Some(clip)) => {
                download::clip_audio_target(&clip, download::audio_format(req.format.as_deref())?)?
            }
            ("video", Some(clip)) if str_at(&clip, "/video_url").is_some() => {
                let mut t = download::clip_audio_target(&clip, "mp3")?;
                t.kind = "video".to_string();
                t.media = MediaKind::Video;
                t.source =
                    DownloadSource::PublicUrl(str_at(&clip, "/video_url").unwrap_or_default());
                t
            }
            ("video", _) => self.music_video_target(&req.id).await?,
            ("image", Some(clip)) => {
                let url = str_at(&clip, "/image_url")
                    .ok_or_else(|| anyhow::anyhow!("Clip {} has no cover image", req.id))?;
                let mut t = download::clip_audio_target(&clip, "mp3")?;
                t.kind = "image".to_string();
                t.media = MediaKind::Image;
                t.source = DownloadSource::PublicUrl(url);
                t
            }
            ("audio" | "image", None) => anyhow::bail!("No clip with ID {}", req.id),
            (other, _) => anyhow::bail!(
                "Unknown download kind '{}'. Use audio, video, or image.",
                other
            ),
        };
        download::download_target(&self.client, &self.db, target).await
    }

    /// Resolves one download record by row `id` or latest `asset_id` for inspection or chunk reads.
    async fn resolve_download_for_get_or_chunk(
        &self,
        id: Option<i64>,
        asset_id: Option<&str>,
        existing_only: bool,
    ) -> Result<DownloadRecord> {
        if let Some(row_id) = id {
            return self
                .db
                .get_download_by_id(row_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("No download record with id {}", row_id));
        }
        let aid = asset_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Specify 'id' or 'asset_id'"))?;
        let rec = if existing_only {
            self.db.find_existing_file_for_asset(aid).await?
        } else {
            self.db.get_latest_download_for_asset(aid).await?
        };
        rec.ok_or_else(|| anyhow::anyhow!("No download record for asset '{}'", aid))
    }

    /// Queries, inspects, reads chunks from, prunes, deletes, or clears indexed downloads.
    pub(crate) async fn downloads(&self, req: DownloadsParams) -> Result<Value> {
        let action = req.action.as_deref().unwrap_or("list");
        match action {
            "list" => {
                let (total_indexed, total_indexed_bytes, downloads) =
                    self.db.list_downloads(req).await?;
                let matched_bytes: u64 = downloads.iter().map(|r| r.size_bytes).sum();
                Ok(json!({
                    "action": "list",
                    "count": downloads.len(),
                    "total_indexed": total_indexed,
                    "matched_bytes": matched_bytes,
                    "total_indexed_bytes": total_indexed_bytes,
                    "db_path": config::db_path().to_string_lossy(),
                    "downloads_dir": config::downloads_dir().to_string_lossy(),
                    "downloads": downloads,
                }))
            }
            "get" => {
                let rec = self
                    .resolve_download_for_get_or_chunk(req.id, req.asset_id.as_deref(), false)
                    .await?;
                Ok(json!({
                    "action": "get",
                    "record": rec,
                }))
            }
            "read_chunk" => {
                let rec = self
                    .resolve_download_for_get_or_chunk(req.id, req.asset_id.as_deref(), true)
                    .await?;
                media_files::read_download_chunk(&rec, req.offset.unwrap_or(0), req.length).await
            }
            "delete" | "remove" => {
                let records = self
                    .db
                    .select_downloads_by_selector(req.id, req.asset_id.as_deref())
                    .await?;
                if records.is_empty() {
                    anyhow::bail!("No download record matches the given selector");
                }
                let (ids, deleted_files, freed_bytes, items) =
                    media_files::remove_managed_download_files(&records).await?;
                let (deleted_records, remaining_records) =
                    self.db.delete_downloads_by_ids(&ids).await?;
                Ok(json!({
                    "action": "delete",
                    "deleted_records": deleted_records,
                    "deleted_files": deleted_files,
                    "freed_bytes": freed_bytes,
                    "remaining_records": remaining_records,
                    "items": items,
                }))
            }
            "prune" => {
                let (pruned_records, remaining_records) = self.db.prune_missing_downloads().await?;
                Ok(json!({
                    "action": "prune",
                    "pruned_records": pruned_records,
                    "remaining_records": remaining_records,
                }))
            }
            "clear" => {
                let records = self
                    .db
                    .select_downloads_for_clear(req.older_than_days)
                    .await?;
                if records.is_empty() {
                    let remaining_records = self.db.count_downloads().await?;
                    return Ok(json!({
                        "action": "clear",
                        "deleted_records": 0,
                        "deleted_files": 0,
                        "freed_bytes": 0,
                        "remaining_records": remaining_records,
                    }));
                }
                let (ids, deleted_files, freed_bytes, _) =
                    media_files::remove_managed_download_files(&records).await?;
                let (deleted_records, remaining_records) =
                    self.db.delete_downloads_by_ids(&ids).await?;
                Ok(json!({
                    "action": "clear",
                    "deleted_records": deleted_records,
                    "deleted_files": deleted_files,
                    "freed_bytes": freed_bytes,
                    "remaining_records": remaining_records,
                }))
            }
            other => anyhow::bail!("Unknown downloads action: {}", other),
        }
    }

    /// Lists all existing downloaded files as MCP [`Resource`] descriptors.
    pub(crate) async fn list_download_resources(&self) -> Result<Vec<Resource>> {
        let records = self.db.list_all_existing_downloads().await?;
        Ok(records.iter().map(record_to_mcp_resource).collect())
    }

    /// Reads one downloaded file addressed by `music://downloads/{id}` or `music://downloads/asset/{asset_id}`.
    pub(crate) async fn read_download_resource(
        &self,
        uri: &str,
    ) -> std::result::Result<ResourceContents, ResourceReadError> {
        let rec = if let Some(asset_id) = uri.strip_prefix(DOWNLOAD_ASSET_RESOURCE_PREFIX) {
            if asset_id.is_empty() || asset_id.contains('/') {
                return Err(ResourceReadError::InvalidUri(format!(
                    "Invalid asset resource URI: {}",
                    uri
                )));
            }
            self.db
                .find_existing_file_for_asset(asset_id)
                .await
                .map_err(|e| ResourceReadError::Internal(e.to_string()))?
                .ok_or_else(|| {
                    ResourceReadError::NotFound(format!(
                        "No downloaded file found for asset '{}'",
                        asset_id
                    ))
                })?
        } else if let Some(id_str) = uri.strip_prefix(DOWNLOAD_RESOURCE_PREFIX) {
            let id = id_str
                .parse::<i64>()
                .ok()
                .filter(|&n| n > 0)
                .ok_or_else(|| {
                    ResourceReadError::InvalidUri(format!("Invalid download resource URI: {}", uri))
                })?;
            self.db
                .get_download_by_id(id)
                .await
                .map_err(|e| ResourceReadError::Internal(e.to_string()))?
                .ok_or_else(|| {
                    ResourceReadError::NotFound(format!("No download record with id {}", id))
                })?
        } else {
            return Err(ResourceReadError::InvalidUri(format!(
                "Unsupported resource URI scheme: {}",
                uri
            )));
        };

        let path =
            media_files::verified_managed_download_path(&rec.file_path).ok_or_else(|| {
                ResourceReadError::NotFound(format!(
                    "Downloaded file is missing from managed downloads directory: {}",
                    rec.file_path
                ))
            })?;
        let meta = tokio::fs::metadata(&path)
            .await
            .map_err(|e| ResourceReadError::Internal(e.to_string()))?;
        if meta.len() > MAX_RESOURCE_READ_BYTES {
            return Err(ResourceReadError::TooLarge(format!(
                "Resource {} is {} bytes (exceeds {} byte limit for resources/read). Use music_downloads with action=\"read_chunk\" and id={}.",
                uri,
                meta.len(),
                MAX_RESOURCE_READ_BYTES,
                rec.id
            )));
        }

        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| ResourceReadError::Internal(e.to_string()))?;
        let mime = effective_mime_type(rec.mime_type.as_deref(), &rec.file_path);
        Ok(ResourceContents::blob(BASE64.encode(&bytes), uri.to_string()).with_mime_type(mime))
    }
}
