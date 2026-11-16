// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Chat messages to the Producer agent and music video jobs.
//!
//! The Producer agent selects and runs tools by itself, thus a chat message can
//! spend credits. A music video takes two direct tool calls in one session:
//! `video__propose_music_video` stores the video inputs, and
//! `video__create_music_video` renders the latest proposal.

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

/// Approximate credits per second of a "music" video, as the page estimates them.
const MUSIC_VIDEO_CREDITS_PER_S: f64 = 12.5;

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
    /// Video type for "create": "music" (default, a custom music video that spends credits) or "social" (a short vertical clip).
    pub(crate) kind: Option<String>,
    /// Video length in seconds (default 15 for "social", 30 for "music"). A "music" video costs about 12.5 credits per second.
    pub(crate) length_s: Option<u32>,
    /// Start point in the song in seconds for a "music" video (default 0).
    pub(crate) start_s: Option<f64>,
    /// Aspect ratio of a "music" video: "9:16" (default) or "16:9".
    pub(crate) aspect_ratio: Option<String>,
    /// Show the lyrics on screen in a "music" video (default false).
    pub(crate) show_lyrics: Option<bool>,
    /// Visual direction for a "music" video, for example the scene and the style.
    pub(crate) instructions: Option<String>,
    /// Style reference image URL for a "music" video, for example the image_url from music_upload.
    pub(crate) style_image_url: Option<String>,
    /// Existing session ID for "create". Omit to create a new session.
    pub(crate) session_id: Option<String>,
    /// Wait for the video to render and download it (default true).
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

/// Waits for the video `video_id`, downloads it when `download` is true, and builds the job result.
async fn finish_video(
    client: &MusicClient,
    db: &MusicDb,
    video_id: String,
    session_id: Option<String>,
    download: bool,
    extra: Value,
    progress: &ProgressCallback,
) -> Result<JobResult> {
    let mut result = JobResult {
        created: vec![video_id.clone()],
        ..JobResult::default()
    };
    let mut payload = json!({ "session_id": session_id, "video_id": video_id });
    if let (Some(dst), Some(src)) = (payload.as_object_mut(), extra.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
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

/// Returns the video ID that a video tool returned, or an error with the tool output.
fn submitted_video_id(content: &Value) -> Result<String> {
    if content.get("needs_credits").and_then(Value::as_bool) == Some(true)
        || content.get("needsCredits").and_then(Value::as_bool) == Some(true)
    {
        anyhow::bail!("Not enough credits for this video: {}", content);
    }
    ["video_id", "videoId", "job_id", "jobId"]
        .iter()
        .find_map(|k| content.get(*k).and_then(Value::as_str))
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("The video tool returned no video ID: {}", content))
}

/// Builds the proposal inputs of a "music" video from `req`.
fn music_video_inputs(req: &VideoParams, clip_id: &str) -> Result<Value> {
    let length = req.length_s.unwrap_or(30);
    anyhow::ensure!(length > 0, "length_s must be greater than 0");
    let start = req.start_s.unwrap_or(0.0);
    anyhow::ensure!(start >= 0.0, "start_s must not be negative");
    let aspect = req.aspect_ratio.as_deref().unwrap_or("9:16");
    anyhow::ensure!(
        matches!(aspect, "9:16" | "16:9"),
        "aspect_ratio must be 9:16 or 16:9"
    );
    Ok(json!({
        "aspect_ratio": aspect,
        "clip_id": clip_id,
        "duration_s": length,
        "render_lyrics": req.show_lyrics.unwrap_or(false),
        "resolution": "720p",
        "start_s": start,
        "style_image_url": req.style_image_url.as_deref().filter(|s| !s.trim().is_empty()),
        "user_message": req.instructions.as_deref().map(str::trim).unwrap_or(""),
    }))
}

/// Returns the credit estimate of a "music" video with `inputs`, as the page computes it.
fn music_video_estimate(inputs: &Value) -> u64 {
    let seconds = inputs
        .get("duration_s")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    // The value is small and positive, thus the conversion cannot lose meaning.
    (seconds * MUSIC_VIDEO_CREDITS_PER_S).ceil().max(0.0) as u64
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
    let download = req.download.unwrap_or(true);
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
            let video_id = submitted_video_id(&outcome.content)?;
            finish_video(
                client,
                db,
                video_id,
                outcome.conversation_id,
                download,
                json!({}),
                &progress,
            )
            .await
        }
        "music" => {
            let inputs = music_video_inputs(&req, &clip_id)?;
            let estimate = music_video_estimate(&inputs);
            progress(JobEvent::Progress(format!(
                "proposing music video (about {estimate} credits)"
            )));
            let proposal = run_tool(
                client,
                ToolRequest {
                    tool_name: "video__propose_music_video",
                    args: json!({ "inputs": inputs }),
                    conversation_id: req.session_id.as_deref(),
                    lyrics_map: Map::new(),
                    timeout: TOOL_TIMEOUT,
                },
                &progress,
            )
            .await?;
            let session = proposal.conversation_id.or(req.session_id.clone());
            // The server takes the inputs of the latest proposal in the session.
            let outcome = run_tool(
                client,
                ToolRequest {
                    tool_name: "video__create_music_video",
                    args: json!({}),
                    conversation_id: session.as_deref(),
                    lyrics_map: Map::new(),
                    timeout: TOOL_TIMEOUT,
                },
                &progress,
            )
            .await?;
            let video_id = submitted_video_id(&outcome.content)?;
            finish_video(
                client,
                db,
                video_id,
                outcome.conversation_id.or(session),
                download,
                json!({ "inputs": inputs, "estimated_credits": estimate }),
                &progress,
            )
            .await
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
