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
use crate::generation::{TOOL_TIMEOUT, finish_clips};
use crate::producer::{ToolRequest, clip_ids_in, run_chat, run_tool};
use crate::queue::{JobResult, ProgressCallback};

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

/// Runs a `music_video` "create" job.
pub(crate) async fn run_video_job(
    client: &MusicClient,
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
                .map(str::to_string);
            Ok(JobResult {
                created: video_id.into_iter().collect(),
                result: Some(json!({
                    "session_id": outcome.conversation_id,
                    "video": outcome.content,
                    "next": "The video renders on the server. Use music_video with action \"list\" or \"status\", then music_download with kind \"video\".",
                })),
                ..JobResult::default()
            })
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
