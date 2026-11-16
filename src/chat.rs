// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Chat messages to the Producer agent and music video jobs.
//!
//! The Producer agent selects and runs tools by itself, thus a chat message can
//! spend credits. Music videos come from the Producer: it first proposes a video
//! plan, and a second message confirms the plan.

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::time::Duration;

use crate::client::MusicClient;
use crate::db::MusicDb;
use crate::download::{download_target, video_target};
use crate::generation::{RENDER_TIMEOUT, TOOL_TIMEOUT, finish_clips};
use crate::producer::{
    READ_ATTEMPTS, READ_RETRY_PAUSE, ToolRequest, clip_ids_in, run_chat, run_tool,
};
use crate::queue::{JobEvent, JobResult, ProgressCallback};

/// Interval between music video status polls.
const VIDEO_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Default maximum time for one Producer reply in seconds.
const DEFAULT_CHAT_TIMEOUT_S: u64 = 900;

/// Parameters for `music_chat`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct ChatParams {
    /// Message to the Producer agent.
    pub(crate) message: String,
    /// Existing session ID. Omit to create a new session.
    pub(crate) session_id: Option<String>,
    /// Maximum wait for the reply in seconds (default 900, range 30 to 3600).
    pub(crate) timeout_secs: Option<u64>,
    /// Download the audio of new clips that the reply creates (default true).
    pub(crate) download: Option<bool>,
    /// Download format: "m4a" (default), "wav", or "mp3".
    pub(crate) format: Option<String>,
}

/// Parameters for `music_video`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct VideoParams {
    /// Action: "list" (default), "get", "status", "delete", or "create" (queued, spends credits).
    pub(crate) action: Option<String>,
    /// Music video ID for "get", "status", and "delete".
    pub(crate) id: Option<String>,
    /// Song clip ID for "create".
    pub(crate) clip_id: Option<String>,
    /// Video type for "create": "music" (default, the Producer plans a full music video) or "social" (a short vertical clip).
    pub(crate) kind: Option<String>,
    /// Length in seconds for a "social" video (default 15).
    pub(crate) length_s: Option<u32>,
    /// Extra direction for a "music" video, for example the visual style.
    pub(crate) instructions: Option<String>,
    /// Existing session ID for "create". Omit to create a new session.
    pub(crate) session_id: Option<String>,
    /// Wait for a "social" video to render and download it (default true).
    pub(crate) download: Option<bool>,
    /// Maximum number of videos for "list" (default 20).
    pub(crate) limit: Option<u32>,
    /// Number of videos to skip for "list" (default 0).
    pub(crate) offset: Option<u32>,
}

/// Runs a `music_chat` job.
pub(crate) async fn run_chat_job(
    client: &MusicClient,
    db: &MusicDb,
    req: ChatParams,
    progress: ProgressCallback,
) -> Result<JobResult> {
    let timeout = Duration::from_secs(
        req.timeout_secs
            .unwrap_or(DEFAULT_CHAT_TIMEOUT_S)
            .clamp(30, 3600),
    );
    let outcome = run_chat(
        client,
        req.session_id.as_deref(),
        &req.message,
        timeout,
        &progress,
    )
    .await?;
    let returns: Vec<&Value> = outcome
        .tools
        .iter()
        .filter(|t| t.get("type").and_then(Value::as_str) == Some("return"))
        .filter_map(|t| t.get("content"))
        .collect();
    let clip_ids = clip_ids_in(&returns);
    let extra = json!({
        "session_id": outcome.conversation_id,
        "reply": outcome.reply.join("\n\n"),
        "title": outcome.title,
        "tools": outcome.tools,
        "suggestions": outcome.suggestions,
    });
    if clip_ids.is_empty() {
        return Ok(JobResult {
            result: Some(extra),
            ..JobResult::default()
        });
    }
    finish_clips(
        client,
        db,
        clip_ids,
        req.download.unwrap_or(true),
        req.format.as_deref(),
        extra,
        &progress,
    )
    .await
}

/// Polls the status of the music video job `video_id` until it completes, then returns its state.
async fn wait_video_ready(
    client: &MusicClient,
    video_id: &str,
    progress: &ProgressCallback,
) -> Result<Value> {
    let deadline = tokio::time::Instant::now() + RENDER_TIMEOUT;
    let mut failures = 0;
    loop {
        match client.get(&format!("/music-video/{video_id}/status")).await {
            Ok(res) => {
                failures = 0;
                let state = res.get("state").cloned().unwrap_or(Value::Null);
                let status = state.get("status").and_then(Value::as_str).unwrap_or("");
                if let Some(err) = state.get("error_message").and_then(Value::as_str) {
                    anyhow::bail!("Music video {} failed: {}", video_id, err);
                }
                match status {
                    "completed" => return Ok(state),
                    "failed" | "error" | "cancelled" => {
                        anyhow::bail!("Music video {} ended with status {}", video_id, status)
                    }
                    _ => {}
                }
                let stage = state
                    .get("current_stage")
                    .and_then(Value::as_str)
                    .unwrap_or("queued");
                progress(JobEvent::Progress(format!("rendering video: {stage}")));
            }
            Err(e) => {
                failures += 1;
                if failures >= READ_ATTEMPTS {
                    return Err(e.context("Polling music video status failed"));
                }
                tokio::time::sleep(READ_RETRY_PAUSE).await;
                continue;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("Timed out waiting for music video {} to render", video_id);
        }
        tokio::time::sleep(VIDEO_POLL_INTERVAL).await;
    }
}

/// Waits for the social video `video_id`, downloads it when `download` is true, and builds the job result.
async fn finish_video(
    client: &MusicClient,
    db: &MusicDb,
    video_id: String,
    session_id: Option<String>,
    download: bool,
    progress: &ProgressCallback,
) -> Result<JobResult> {
    let mut result = JobResult {
        created: vec![video_id.clone()],
        ..JobResult::default()
    };
    let mut payload = json!({ "session_id": session_id, "video_id": video_id });
    if !download {
        payload["next"] = json!(
            "The video renders on the server. Use music_video with action \"status\", then music_download with kind \"video\"."
        );
        result.result = Some(payload);
        return Ok(result);
    }
    let state = wait_video_ready(client, &video_id, progress).await?;
    payload["video_url"] = state.get("final_video_url").cloned().unwrap_or(Value::Null);
    payload["duration_s"] = state
        .get("video_duration_s")
        .cloned()
        .unwrap_or(Value::Null);
    payload["start_s"] = state.get("video_start_s").cloned().unwrap_or(Value::Null);
    match state.get("final_video_url").and_then(Value::as_str) {
        Some(url) => {
            progress(JobEvent::Progress("downloading video".to_string()));
            let target = video_target(&video_id, url.to_string(), session_id, None);
            match download_target(client, db, target).await {
                Ok(rec) => {
                    result.files.push(rec.file_path);
                    result.resources.push(rec.resource_uri);
                }
                Err(e) => payload["download_errors"] = json!([format!("{e:#}")]),
            }
        }
        None => payload["download_errors"] = json!(["The finished video has no file URL"]),
    }
    result.result = Some(payload);
    Ok(result)
}

/// Runs a `music_video` "create" job.
pub(crate) async fn run_video_job(
    client: &MusicClient,
    db: &MusicDb,
    req: VideoParams,
    progress: ProgressCallback,
) -> Result<JobResult> {
    let clip_id = req
        .clip_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Action \"create\" needs 'clip_id'"))?;
    match req.kind.as_deref().unwrap_or("music") {
        "social" => {
            let outcome = run_tool(
                client,
                ToolRequest {
                    tool_name: "video__create_social_video",
                    args: json!({ "clip_id": clip_id, "length_s": req.length_s.unwrap_or(15) }),
                    conversation_id: req.session_id.as_deref(),
                    lyrics_map: Map::new(),
                    timeout: TOOL_TIMEOUT,
                },
                &progress,
            )
            .await?;
            let video_id = ["video_id", "videoId", "job_id"]
                .iter()
                .find_map(|k| outcome.content.get(*k).and_then(Value::as_str))
                .map(str::to_string)
                .ok_or_else(|| {
                    anyhow::anyhow!("The video tool returned no video ID: {}", outcome.content)
                })?;
            finish_video(
                client,
                db,
                video_id,
                outcome.conversation_id,
                req.download.unwrap_or(true),
                &progress,
            )
            .await
        }
        "music" => {
            let extra = req
                .instructions
                .as_deref()
                .map(|s| format!(" Direction: {s}"))
                .unwrap_or_default();
            let message = format!(
                "Make a music video for the song with clip ID {clip_id}.{extra} Use the song as the soundtrack."
            );
            let outcome = run_chat(
                client,
                req.session_id.as_deref(),
                &message,
                Duration::from_secs(DEFAULT_CHAT_TIMEOUT_S),
                &progress,
            )
            .await?;
            Ok(JobResult {
                result: Some(json!({
                    "session_id": outcome.conversation_id,
                    "reply": outcome.reply.join("\n\n"),
                    "tools": outcome.tools,
                    "suggestions": outcome.suggestions,
                    "next": "If the Producer proposed a plan, confirm it with music_chat in the same session. Then use music_video with action \"list\".",
                })),
                ..JobResult::default()
            })
        }
        other => anyhow::bail!("Unknown video kind '{}'. Use music or social.", other),
    }
}

/// Runs the immediate `music_video` actions: "list", "get", "status", and "delete".
pub(crate) async fn video_library(client: &MusicClient, req: &VideoParams) -> Result<Value> {
    let id = || {
        req.id
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("This action needs 'id'"))
    };
    match req.action.as_deref().unwrap_or("list") {
        "list" => {
            let path = format!(
                "/music-videos/get?offset={}&limit={}",
                req.offset.unwrap_or(0),
                req.limit.unwrap_or(20).clamp(1, 100)
            );
            client.get(&path).await
        }
        "get" => client.get(&format!("/music-video/get/{}", id()?)).await,
        "status" => client.get(&format!("/music-video/{}/status", id()?)).await,
        "delete" => {
            client
                .api("DELETE", &format!("/music-video/{}", id()?), None)
                .await
        }
        other => anyhow::bail!("Unknown video action: {}", other),
    }
}
