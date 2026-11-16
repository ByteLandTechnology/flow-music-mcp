// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Media downloads into `<home>/downloads` and SQLite indexing.
//!
//! Song, stem, and cover files have public storage URLs, thus the server streams
//! them directly. MP3 conversions need the session, thus the page runtime fetches
//! them through the internal API.

use anyhow::{Context, Result};
use std::path::Path;
use std::sync::OnceLock;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::time::Duration;

use crate::client::MusicClient;
use crate::config::downloads_dir;
use crate::db::{DownloadRecord, MusicDb, NewDownloadRecord};
use crate::media_files::{
    MediaKind, disambiguate_dest, natural_filename, resolve_stored_path, validate_downloaded_file,
};
use crate::private_fs::create_private_dir_all;

/// Location of the bytes of one download.
#[derive(Debug, Clone)]
pub(crate) enum DownloadSource {
    /// Public HTTPS URL that needs no session.
    PublicUrl(String),
    /// Internal API path that needs the session of the page.
    Api(String),
}

/// One file to download and index.
#[derive(Debug, Clone)]
pub(crate) struct DownloadTarget {
    /// Google Flow Music clip or video ID.
    pub(crate) id: String,
    /// Display name, for example the song title.
    pub(crate) name: String,
    /// Index kind: `"audio"`, `"stem"`, `"video"`, or `"image"`.
    pub(crate) kind: String,
    /// Media family that the file must contain.
    pub(crate) media: MediaKind,
    /// Sound prompt or other prompt text, if known.
    pub(crate) prompt: Option<String>,
    /// Session (conversation) ID, if known.
    pub(crate) session_id: Option<String>,
    /// Session title, if known.
    pub(crate) session_title: Option<String>,
    /// Creation time of the clip, if known.
    pub(crate) created_at: Option<String>,
    /// Location of the bytes.
    pub(crate) source: DownloadSource,
}

/// Header values and size of a staged download.
struct Staged {
    /// Value of the `Content-Type` header.
    content_type: Option<String>,
    /// Value of the `Content-Disposition` header.
    disposition: Option<String>,
    /// Number of bytes written.
    size: u64,
}

/// Returns the shared HTTP client for public media downloads.
fn download_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Returns a header value as an owned string.
fn header_value(resp: &reqwest::Response, name: reqwest::header::HeaderName) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Streams a public URL to `temp_path`.
async fn stream_public(client: &MusicClient, url: &str, temp_path: &Path) -> Result<Staged> {
    let ua = client.engine().user_agent().await;
    let mut resp = download_client()
        .get(url)
        .header(reqwest::header::USER_AGENT, ua)
        .send()
        .await
        .with_context(|| format!("Request to {url} failed"))?;
    if !resp.status().is_success() {
        anyhow::bail!("{} returned HTTP {}", url, resp.status());
    }
    let content_type = header_value(&resp, reqwest::header::CONTENT_TYPE);
    let disposition = header_value(&resp, reqwest::header::CONTENT_DISPOSITION);
    let file = tokio::fs::File::create(temp_path).await?;
    let mut writer = BufWriter::new(file);
    let mut size = 0u64;
    while let Some(chunk) = resp.chunk().await? {
        size += chunk.len() as u64;
        writer.write_all(&chunk).await?;
    }
    writer.flush().await?;
    Ok(Staged {
        content_type,
        disposition,
        size,
    })
}

/// Fetches an API resource through the page runtime and writes it to `temp_path`.
async fn fetch_api(client: &MusicClient, path: &str, temp_path: &Path) -> Result<Staged> {
    let res = client.fetch_binary(path).await?;
    tokio::fs::write(temp_path, &res.bytes).await?;
    Ok(Staged {
        content_type: res.content_type,
        disposition: res.disposition,
        size: res.bytes.len() as u64,
    })
}

/// Downloads `target` into `<home>/downloads` and indexes it in SQLite.
pub(crate) async fn download_target(
    client: &MusicClient,
    db: &MusicDb,
    target: DownloadTarget,
) -> Result<DownloadRecord> {
    let raw_dir = downloads_dir();
    create_private_dir_all(&raw_dir)?;
    let stage_dir = std::fs::canonicalize(&raw_dir).unwrap_or(raw_dir);
    let temp_name = format!(
        ".flow-music-mcp-{}.part",
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let temp_path = stage_dir.join(temp_name);
    let (staged, source_url) = match &target.source {
        DownloadSource::PublicUrl(url) => {
            (stream_public(client, url, &temp_path).await, url.clone())
        }
        DownloadSource::Api(path) => (
            fetch_api(client, path, &temp_path).await,
            format!("{}/__api{}", crate::config::APP_ORIGIN, path),
        ),
    };
    let staged = match staged {
        Ok(s) => s,
        Err(e) => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(e.context(format!("Failed to download '{}'", target.name)));
        }
    };
    let mime =
        match validate_downloaded_file(&temp_path, staged.content_type.as_deref(), target.media)
            .await
        {
            Ok(m) => m,
            Err(e) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return Err(e);
            }
        };
    let filename = natural_filename(
        &target.id,
        target.media,
        &source_url,
        Some(mime.as_str()),
        staged.disposition.as_deref(),
    );
    let dest = disambiguate_dest(db, &stage_dir, &filename, &target.id).await;
    if let Err(e) = tokio::fs::rename(&temp_path, &dest).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(e).with_context(|| format!("Failed to save download to {}", dest.display()));
    }
    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&filename)
        .to_string();
    db.record_download(NewDownloadRecord {
        asset_id: target.id,
        session_id: target.session_id,
        session_title: target.session_title,
        name: target.name,
        kind: target.kind,
        prompt: target.prompt,
        source_url: Some(source_url),
        file_path: resolve_stored_path(&dest),
        file_name,
        mime_type: Some(mime),
        size_bytes: staged.size,
        asset_created_at: target.created_at,
    })
    .await
}

/// Audio formats that `music_download` and queued jobs can save.
pub(crate) const AUDIO_FORMATS: [&str; 3] = ["m4a", "wav", "mp3"];

/// Returns the normalized audio format, or an error for an unknown format.
pub(crate) fn audio_format(raw: Option<&str>) -> Result<&'static str> {
    let wanted = raw.unwrap_or("m4a").trim().to_ascii_lowercase();
    AUDIO_FORMATS
        .iter()
        .copied()
        .find(|f| *f == wanted)
        .ok_or_else(|| anyhow::anyhow!("Unknown audio format '{}'. Use m4a, wav, or mp3.", wanted))
}

/// Returns a non-empty string field of a clip record.
fn clip_str(clip: &serde_json::Value, pointer: &str) -> Option<String> {
    clip.pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Builds the download target for the audio of `clip` in `format` (`m4a`, `wav`, or `mp3`).
pub(crate) fn clip_audio_target(clip: &serde_json::Value, format: &str) -> Result<DownloadTarget> {
    let id = clip_str(clip, "/id").ok_or_else(|| anyhow::anyhow!("Clip record has no id"))?;
    let source = match format {
        "wav" => DownloadSource::PublicUrl(
            clip_str(clip, "/wav_url")
                .ok_or_else(|| anyhow::anyhow!("Clip {} has no WAV file yet", id))?,
        ),
        "mp3" => DownloadSource::Api(format!("/download/audio/{id}?format=mp3")),
        _ => DownloadSource::PublicUrl(
            clip_str(clip, "/audio_url")
                .ok_or_else(|| anyhow::anyhow!("Clip {} has no audio file yet", id))?,
        ),
    };
    let is_stem = clip_str(clip, "/op_type").as_deref() == Some("audio__split_stems");
    Ok(DownloadTarget {
        name: clip_str(clip, "/title").unwrap_or_else(|| id.clone()),
        id,
        kind: if is_stem { "stem" } else { "audio" }.to_string(),
        media: MediaKind::Audio,
        prompt: clip_str(clip, "/operation/sound_prompt"),
        session_id: clip_str(clip, "/operation/conversation_id"),
        session_title: None,
        created_at: clip_str(clip, "/created_at"),
        source,
    })
}

/// Files, resource URIs, and error texts of a batch of downloads.
#[derive(Debug, Default)]
pub(crate) struct BatchDownload {
    /// Local file paths of the saved files.
    pub(crate) files: Vec<String>,
    /// MCP resource URIs of the saved files.
    pub(crate) resources: Vec<String>,
    /// Error texts of failed downloads.
    pub(crate) errors: Vec<String>,
}

/// Downloads the audio of every clip in `clips` in `format` and continues after failures.
pub(crate) async fn download_clips(
    client: &MusicClient,
    db: &MusicDb,
    clips: &[serde_json::Value],
    format: &str,
) -> BatchDownload {
    let mut out = BatchDownload::default();
    for clip in clips {
        let res = match clip_audio_target(clip, format) {
            Ok(target) => download_target(client, db, target).await,
            Err(e) => Err(e),
        };
        match res {
            Ok(rec) => {
                out.files.push(rec.file_path);
                out.resources.push(rec.resource_uri);
            }
            Err(e) => out.errors.push(format!("{e:#}")),
        }
    }
    out
}
