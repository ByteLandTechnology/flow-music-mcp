// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Song generation and lyrics jobs.
//!
//! The page builds the `audio__create_song` arguments from the compose form. This
//! module builds the same arguments: BPM goes into the sound prompt, the length goes
//! into the lyrics as an `[End - m:ss]` marker, and the lyrics identifier is the
//! UUID version 5 of the lyrics text. The model treats lyrics, BPM, and length as
//! guidance, thus the result can differ from the request.

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::time::Duration;

use crate::client::MusicClient;
use crate::db::MusicDb;
use crate::download::{audio_format, download_clips};
use crate::producer::{ToolRequest, clip_ids_in, clip_summary, run_tool, wait_clips_ready};
use crate::queue::{JobEvent, JobResult, ProgressCallback};

/// Namespace UUID that the page uses for lyrics identifiers.
const LYRICS_NAMESPACE: &str = "b8f9e3a1-7c2d-4f5e-9a8b-1c3d5e7f9a2b";

/// Lyrics text that marks an instrumental song.
const INSTRUMENTAL: &str = "[Instrumental]";

/// Maximum time for a Producer tool call to return.
pub(crate) const TOOL_TIMEOUT: Duration = Duration::from_secs(600);

/// Maximum time for created clips to finish rendering.
pub(crate) const RENDER_TIMEOUT: Duration = Duration::from_secs(900);

/// Parameters for `music_generate`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct GenerateParams {
    /// Sound description: genre, instruments, mood, and vocal style.
    pub(crate) prompt: String,
    /// Lyrics text. The model uses the lyrics as guidance and can change them. Omit for model-written lyrics.
    pub(crate) lyrics: Option<String>,
    /// Lyrics ID from `music_lyrics`. When set, the tool ignores `lyrics` and `length_s`.
    pub(crate) lyrics_id: Option<String>,
    /// Make an instrumental song without vocals (default false).
    pub(crate) instrumental: Option<bool>,
    /// Tempo hint in beats per minute. The value goes into the sound prompt.
    pub(crate) bpm: Option<u32>,
    /// Length hint in seconds. The model can make a different length.
    pub(crate) length_s: Option<u32>,
    /// Song title (default "Untitled").
    pub(crate) title: Option<String>,
    /// Seed for repeatable results.
    pub(crate) seed: Option<i64>,
    /// Image ID from `music_upload` to use as the prompt image.
    pub(crate) image_id: Option<String>,
    /// Existing session ID. Omit to create a new session.
    pub(crate) session_id: Option<String>,
    /// Download the finished audio (default true).
    pub(crate) download: Option<bool>,
    /// Download format: "m4a" (default), "wav", or "mp3".
    pub(crate) format: Option<String>,
}

/// Parameters for `music_lyrics`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct LyricsParams {
    /// Action: "write" (default) makes new lyrics from `prompt`; "edit" changes `lyrics` or `lyrics_id` with `prompt`.
    pub(crate) action: Option<String>,
    /// Instruction for the lyrics writer.
    pub(crate) prompt: String,
    /// Existing lyrics text to edit for "edit".
    pub(crate) lyrics: Option<String>,
    /// Existing lyrics ID to edit for "edit".
    pub(crate) lyrics_id: Option<String>,
    /// Existing session ID. Omit to create a new session.
    pub(crate) session_id: Option<String>,
}

/// Arguments for `audio__create_song` and the lyrics map for the client context.
struct CreateSongArgs {
    /// Tool arguments.
    args: Value,
    /// Lyrics text keyed by lyrics ID.
    lyrics_map: Map<String, Value>,
}

/// Formats `seconds` as `m:ss`.
fn format_length(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Builds the `audio__create_song` arguments in the same way as the compose form of the page.
async fn build_create_args(client: &MusicClient, req: &GenerateParams) -> Result<CreateSongArgs> {
    let instrumental = req.instrumental.unwrap_or(false);
    let user_lyrics = req.lyrics.as_deref().map(str::trim).unwrap_or("");
    let mut lyrics = if instrumental {
        INSTRUMENTAL.to_string()
    } else {
        user_lyrics.to_string()
    };
    if let Some(len) = req.length_s.filter(|l| *l > 0) {
        let marker = format!("[End - {}]", format_length(len));
        lyrics = if lyrics.is_empty() {
            marker
        } else {
            format!("{lyrics}\n{marker}")
        };
    }
    let mut sound = req.prompt.trim().to_string();
    if let Some(bpm) = req.bpm.filter(|b| *b > 0) {
        sound = if sound.is_empty() {
            format!("{bpm} bpm")
        } else {
            format!("{sound}, {bpm} bpm")
        };
    }
    anyhow::ensure!(
        !sound.is_empty() || !lyrics.is_empty(),
        "Give a sound prompt or lyrics"
    );
    let mut lyrics_map = Map::new();
    let given_id = req
        .lyrics_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !instrumental);
    let lyrics_id = if let Some(id) = given_id {
        id.to_string()
    } else if lyrics.is_empty() || lyrics == INSTRUMENTAL {
        String::new()
    } else {
        let id = client.uuid_v5(&lyrics, LYRICS_NAMESPACE).await?;
        lyrics_map.insert(id.clone(), json!(lyrics));
        id
    };
    Ok(CreateSongArgs {
        args: json!({
            "sound_prompt": sound,
            "lyrics_id": lyrics_id,
            "title": req.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("Untitled"),
            "seed": req.seed,
            "image_id": req.image_id,
        }),
        lyrics_map,
    })
}

/// Waits for `clip_ids`, downloads them when asked, and builds the job result.
pub(crate) async fn finish_clips(
    client: &MusicClient,
    db: &MusicDb,
    clip_ids: Vec<String>,
    download: bool,
    format: Option<&str>,
    extra: Value,
    progress: &ProgressCallback,
) -> Result<JobResult> {
    anyhow::ensure!(!clip_ids.is_empty(), "The tool returned no clip IDs");
    let clips = wait_clips_ready(client, &clip_ids, RENDER_TIMEOUT, progress).await?;
    let mut result = JobResult {
        created: clip_ids,
        ..JobResult::default()
    };
    let mut payload = json!({
        "clips": clips.iter().map(clip_summary).collect::<Vec<_>>(),
    });
    if download {
        let fmt = audio_format(format)?;
        progress(JobEvent::Progress(format!("downloading {fmt}")));
        let batch = download_clips(client, db, &clips, fmt).await;
        if !batch.errors.is_empty() {
            progress(JobEvent::Progress(format!(
                "completed with download error(s): {}",
                batch.errors.join(" | ")
            )));
            payload["download_errors"] = json!(batch.errors);
        }
        result.files = batch.files;
        result.resources = batch.resources;
    }
    if let (Some(dst), Some(src)) = (payload.as_object_mut(), extra.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    result.result = Some(payload);
    Ok(result)
}

/// Runs a `music_generate` job.
pub(crate) async fn run_generate(
    client: &MusicClient,
    db: &MusicDb,
    req: GenerateParams,
    progress: ProgressCallback,
) -> Result<JobResult> {
    let built = build_create_args(client, &req).await?;
    let outcome = run_tool(
        client,
        ToolRequest {
            tool_name: "audio__create_song",
            args: built.args,
            conversation_id: req.session_id.as_deref(),
            lyrics_map: built.lyrics_map,
            timeout: TOOL_TIMEOUT,
        },
        &progress,
    )
    .await?;
    let ids = clip_ids_in(&[&outcome.content]);
    finish_clips(
        client,
        db,
        ids,
        req.download.unwrap_or(true),
        req.format.as_deref(),
        json!({ "session_id": outcome.conversation_id }),
        &progress,
    )
    .await
}

/// Runs a `music_lyrics` job.
pub(crate) async fn run_lyrics(
    client: &MusicClient,
    req: LyricsParams,
    progress: ProgressCallback,
) -> Result<JobResult> {
    let mut conversation = req.session_id.clone();
    let content = match req.action.as_deref().unwrap_or("write") {
        "write" | "create" => {
            let out = run_tool(
                client,
                ToolRequest {
                    tool_name: "lyrics__create",
                    args: json!({ "prompt": req.prompt }),
                    conversation_id: conversation.as_deref(),
                    lyrics_map: Map::new(),
                    timeout: TOOL_TIMEOUT,
                },
                &progress,
            )
            .await?;
            conversation = out.conversation_id.or(conversation);
            out.content
        }
        "edit" => {
            let lyrics_id = match (req.lyrics_id.clone(), req.lyrics.as_deref()) {
                (Some(id), _) => id,
                (None, Some(text)) => {
                    let reg = run_tool(
                        client,
                        ToolRequest {
                            tool_name: "lyrics__register",
                            args: json!({ "lyrics": text }),
                            conversation_id: conversation.as_deref(),
                            lyrics_map: Map::new(),
                            timeout: TOOL_TIMEOUT,
                        },
                        &progress,
                    )
                    .await?;
                    conversation = reg.conversation_id.or(conversation);
                    reg.content
                        .get("lyrics_id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("Lyrics registration returned no ID"))?
                        .to_string()
                }
                (None, None) => anyhow::bail!("Action \"edit\" needs 'lyrics' or 'lyrics_id'"),
            };
            let out = run_tool(
                client,
                ToolRequest {
                    tool_name: "lyrics__edit",
                    args: json!({ "lyrics_id": lyrics_id, "prompt": req.prompt }),
                    conversation_id: conversation.as_deref(),
                    lyrics_map: Map::new(),
                    timeout: TOOL_TIMEOUT,
                },
                &progress,
            )
            .await?;
            conversation = out.conversation_id.or(conversation);
            out.content
        }
        other => anyhow::bail!("Unknown lyrics action: {}", other),
    };
    let lyrics_id = content
        .get("lyrics_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(JobResult {
        created: lyrics_id.into_iter().collect(),
        result: Some(json!({
            "title": content.get("title"),
            "lyrics": content.get("lyrics"),
            "sound_prompt": content.get("sound_prompt"),
            "lyrics_id": content.get("lyrics_id"),
            "session_id": conversation,
        })),
        ..JobResult::default()
    })
}
