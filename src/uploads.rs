// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Uploads of local audio and image files to Google Flow Music.
//!
//! An uploaded audio file becomes a clip that the edit tools can use. An uploaded
//! image gives an image ID for the prompt image of `music_generate`.

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tokio::time::Instant;

use crate::client::MusicClient;

/// Maximum audio upload size that the page accepts (40 MiB).
const MAX_AUDIO_BYTES: u64 = 40 * 1024 * 1024;

/// Maximum image upload size that the page accepts (5 MiB).
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

/// Maximum time for the server to check an uploaded audio file.
const AUDIO_CHECK_TIMEOUT: Duration = Duration::from_secs(300);

/// Parameters for `music_upload`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct UploadParams {
    /// Absolute path of a local audio file (under 4 minutes and 40 MiB) or image file (under 5 MiB).
    pub(crate) path: String,
    /// File kind: "audio" or "image". Default: from the file type.
    pub(crate) kind: Option<String>,
}

/// Returns the upload kind of `mime`, or `None` for an unsupported type.
fn kind_of(mime: &str) -> Option<&'static str> {
    if mime.starts_with("audio/") {
        Some("audio")
    } else if mime.starts_with("image/") {
        Some("image")
    } else {
        None
    }
}

/// Polls the server check of an uploaded audio clip until it finishes.
async fn wait_audio_check(
    client: &MusicClient,
    clip_id: &str,
    operation_id: &str,
) -> Result<Value> {
    let deadline = Instant::now() + AUDIO_CHECK_TIMEOUT;
    loop {
        let status = client
            .get(&format!(
                "/producer/upload-audio/{clip_id}/upload-check-status?operation_id={operation_id}"
            ))
            .await?;
        match status.get("status").and_then(Value::as_str).unwrap_or("") {
            "complete" | "completed" | "success" | "succeeded" => return Ok(status),
            "failed" | "error" | "rejected" => {
                anyhow::bail!("The server rejected the uploaded audio: {}", status)
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            anyhow::bail!("Timed out waiting for the server to check the uploaded audio");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Uploads a local audio or image file.
pub(crate) async fn upload(client: &MusicClient, req: UploadParams) -> Result<Value> {
    let path = Path::new(&req.path);
    let meta = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("Cannot read {}", req.path))?;
    anyhow::ensure!(meta.is_file(), "{} is not a file", req.path);
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_string();
    let kind = match req.kind.as_deref() {
        Some(k @ ("audio" | "image")) => k,
        Some(other) => anyhow::bail!("Unknown upload kind '{}'. Use audio or image.", other),
        None => kind_of(&mime)
            .ok_or_else(|| anyhow::anyhow!("Cannot tell the kind of {}; set 'kind'", req.path))?,
    };
    let limit = if kind == "audio" {
        MAX_AUDIO_BYTES
    } else {
        MAX_IMAGE_BYTES
    };
    anyhow::ensure!(
        meta.len() <= limit,
        "{} is {} bytes; the limit for {} uploads is {} bytes",
        req.path,
        meta.len(),
        kind,
        limit
    );
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("upload")
        .to_string();
    let bytes = tokio::fs::read(path).await?;
    if kind == "image" {
        let res = client
            .upload(
                "/image/upload",
                "image",
                &file_name,
                &mime,
                &bytes,
                json!({ "image_type": "clip" }),
            )
            .await?;
        return Ok(json!({
            "kind": "image",
            "file": file_name,
            "image_id": res.get("image_id"),
            "image_url": res.get("image_url"),
        }));
    }
    let res = client
        .upload(
            "/producer/upload-audio",
            "file",
            &file_name,
            &mime,
            &bytes,
            json!({ "file_type": mime, "filename": file_name, "selected_model": "Lyria 3.5" }),
        )
        .await?;
    let clip_id = res
        .get("clip_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Audio upload returned no clip_id: {}", res))?
        .to_string();
    let check = match res.get("operation_id").and_then(Value::as_str) {
        Some(op) => wait_audio_check(client, &clip_id, op).await?,
        None => Value::Null,
    };
    Ok(json!({ "kind": "audio", "file": file_name, "clip_id": clip_id, "check": check }))
}
