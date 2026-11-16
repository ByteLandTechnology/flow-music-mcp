// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Producer tool calls, chat messages, job streams, and clip completion polling.
//!
//! A Producer tool call returns a job ID. The job stream sends the tool call, then the
//! tool return with the result. A song can still render after the tool return, thus
//! [`wait_clips_ready`] polls the clip records until their audio is complete.

use anyhow::Result;
use serde_json::{Map, Value, json};
use std::time::Duration;
use tokio::time::Instant;

use crate::client::{MusicClient, StreamEvent};
use crate::queue::{JobEvent, ProgressCallback};

/// Maximum duration of one bounded stream read in milliseconds.
const STREAM_SLICE_MS: u64 = 20_000;

/// Interval between clip status polls.
const CLIP_POLL_INTERVAL: Duration = Duration::from_secs(4);

/// Request for one direct Producer tool call.
pub(crate) struct ToolRequest<'a> {
    /// Producer tool name, for example `audio__create_song`.
    pub(crate) tool_name: &'a str,
    /// Tool arguments.
    pub(crate) args: Value,
    /// Existing conversation (session) ID, or `None` for a new conversation.
    pub(crate) conversation_id: Option<&'a str>,
    /// Lyrics text keyed by lyrics ID, sent in the client context.
    pub(crate) lyrics_map: Map<String, Value>,
    /// Maximum time to wait for the tool return.
    pub(crate) timeout: Duration,
}

/// Result of a finished Producer tool call.
#[derive(Debug, Clone)]
pub(crate) struct ToolOutcome {
    /// Conversation (session) ID that the server used.
    pub(crate) conversation_id: Option<String>,
    /// Content of the tool return part.
    pub(crate) content: Value,
}

/// Result of a finished chat message.
#[derive(Debug, Clone, Default)]
pub(crate) struct ChatOutcome {
    /// Conversation (session) ID that the server used.
    pub(crate) conversation_id: Option<String>,
    /// Final text parts of the Producer reply.
    pub(crate) reply: Vec<String>,
    /// Tool calls and tool returns that the Producer made, in order.
    pub(crate) tools: Vec<Value>,
    /// Suggested next actions from the Producer, if any.
    pub(crate) suggestions: Vec<String>,
    /// Generated conversation title, if any.
    pub(crate) title: Option<String>,
}

/// Builds the client context object that the page sends with Producer requests.
fn client_context(lyrics_map: Map<String, Value>) -> Value {
    json!({
        "song_queue": [],
        "selected_model": "Lyria 3.5",
        "lyrics_id_map": lyrics_map,
        "ghostwriter_version": "standard",
        "onboarding_variant": null,
        "client_platform": "web",
        "clarifying_questions": {
            "supports_question_options": false,
            "supports_multi_question_options": false
        }
    })
}

/// Returns the parts of a stream event: `data.parts` for messages, or `[data.part]` for chat parts.
fn event_parts(event: &StreamEvent) -> Vec<Value> {
    if let Some(parts) = event.data.get("parts").and_then(Value::as_array) {
        return parts.clone();
    }
    if event.data.get("status").and_then(Value::as_str) == Some("final")
        && let Some(part) = event.data.get("part")
    {
        return vec![part.clone()];
    }
    Vec::new()
}

/// Formats the error text of a stream `error` event.
fn stream_error(event: &StreamEvent) -> String {
    let message = event
        .data
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Producer job failed");
    let kind = event.data.get("type").and_then(Value::as_str).unwrap_or("");
    if kind.is_empty() {
        message.to_string()
    } else {
        format!("{message} ({kind})")
    }
}

/// Fails when tool return `content` reports a status other than success.
fn check_tool_content(tool_name: &str, content: &Value) -> Result<()> {
    let status = content
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("success");
    if status == "success" || status == "complete" {
        return Ok(());
    }
    let detail = content
        .get("error")
        .or_else(|| content.get("message"))
        .map(Value::to_string)
        .unwrap_or_else(|| content.to_string());
    anyhow::bail!(
        "Producer tool {} returned status {}: {}",
        tool_name,
        status,
        detail
    )
}

/// Sends a Producer tool call and waits for its tool return.
pub(crate) async fn run_tool(
    client: &MusicClient,
    req: ToolRequest<'_>,
    progress: &ProgressCallback,
) -> Result<ToolOutcome> {
    let tool_call_id = uuid::Uuid::new_v4().to_string();
    let mut body = json!({
        "part": {
            "tool_name": req.tool_name,
            "args": req.args,
            "part_kind": "tool-call",
            "tool_call_id": tool_call_id,
        },
        "client_context": client_context(req.lyrics_map),
    });
    if let (Some(conv), Some(obj)) = (req.conversation_id, body.as_object_mut()) {
        obj.insert("conversation_id".to_string(), json!(conv));
    }
    let started = client.post("/producer/tool-call", body).await?;
    let job_id = started
        .get("job_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Producer tool call returned no job_id: {}", started))?
        .to_string();
    progress(JobEvent::Progress(format!("{} started", req.tool_name)));

    let deadline = Instant::now() + req.timeout;
    let mut last_id = "0".to_string();
    let mut conversation_id = req.conversation_id.map(str::to_string);
    loop {
        let chunk = client
            .read_stream(&job_id, &last_id, STREAM_SLICE_MS, false)
            .await?;
        last_id = chunk.last_id.clone();
        for event in &chunk.events {
            match event.event.as_str() {
                "conversation_id" => {
                    if let Some(id) = event.data.get("id").and_then(Value::as_str) {
                        conversation_id = Some(id.to_string());
                        progress(JobEvent::Conversation(id.to_string()));
                    }
                }
                "error" => anyhow::bail!("{}", stream_error(event)),
                _ => {}
            }
            for part in event_parts(event) {
                let is_return =
                    part.get("part_kind").and_then(Value::as_str) == Some("tool-return");
                let same_call =
                    part.get("tool_call_id").and_then(Value::as_str) == Some(tool_call_id.as_str());
                if is_return && same_call {
                    let content = part.get("content").cloned().unwrap_or(Value::Null);
                    check_tool_content(req.tool_name, &content)?;
                    return Ok(ToolOutcome {
                        conversation_id,
                        content,
                    });
                }
            }
        }
        if chunk.done {
            anyhow::bail!(
                "Producer job {} ended without a result for {}",
                job_id,
                req.tool_name
            );
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "Timed out waiting for {} (job {}). The job can still finish on the server.",
                req.tool_name,
                job_id
            );
        }
    }
}

/// Sends a chat message to the Producer and collects the reply until the job finishes.
pub(crate) async fn run_chat(
    client: &MusicClient,
    conversation_id: Option<&str>,
    message: &str,
    timeout: Duration,
    progress: &ProgressCallback,
) -> Result<ChatOutcome> {
    let conv = match conversation_id {
        Some(id) => id.to_string(),
        None => {
            let created = client
                .post("/conversation/create", json!({ "project_id": null }))
                .await?;
            created
                .get("conversation_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("Session creation returned no ID: {}", created))?
                .to_string()
        }
    };
    progress(JobEvent::Conversation(conv.clone()));
    let body = json!({
        "conversation_id": conv,
        "parts": [{ "part_kind": "user-prompt", "content": message }],
        "client_context": client_context(Map::new()),
        "mode": "standard",
    });
    let started = client.post("/conversation", body).await?;
    let job_id = started
        .get("job_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Chat message returned no job_id: {}", started))?
        .to_string();
    progress(JobEvent::Progress("Producer is working".to_string()));

    let mut out = ChatOutcome {
        conversation_id: Some(conv),
        ..ChatOutcome::default()
    };
    let deadline = Instant::now() + timeout;
    let mut last_id = "0".to_string();
    loop {
        let chunk = client
            .read_stream(&job_id, &last_id, STREAM_SLICE_MS, true)
            .await?;
        last_id = chunk.last_id.clone();
        for event in &chunk.events {
            collect_chat_event(event, &mut out, progress)?;
        }
        if chunk.done {
            return Ok(out);
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "Timed out waiting for the Producer reply (job {}). The job can still finish on the server.",
                job_id
            );
        }
    }
}

/// Adds the content of one chat stream event to `out`.
fn collect_chat_event(
    event: &StreamEvent,
    out: &mut ChatOutcome,
    progress: &ProgressCallback,
) -> Result<()> {
    match event.event.as_str() {
        "error" => anyhow::bail!("{}", stream_error(event)),
        "generated-title" => {
            out.title = event
                .data
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string);
            return Ok(());
        }
        "suggestion" => {
            for part in event_parts(event) {
                if part.get("tool_name").and_then(Value::as_str)
                    == Some("synthetic__suggest_actions")
                    && let Some(args) = part.get("args").and_then(Value::as_object)
                {
                    out.suggestions
                        .extend(args.values().filter_map(Value::as_str).map(str::to_string));
                }
            }
            return Ok(());
        }
        "part" => {}
        _ => return Ok(()),
    }
    for part in event_parts(event) {
        match part.get("part_kind").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = part.get("content").and_then(Value::as_str) {
                    out.reply.push(text.to_string());
                }
            }
            Some("tool-call") => {
                let name = part.get("tool_name").and_then(Value::as_str).unwrap_or("");
                progress(JobEvent::Progress(format!("Producer calls {name}")));
                out.tools.push(json!({
                    "type": "call",
                    "tool": name,
                    "args": part.get("args").cloned().unwrap_or(Value::Null),
                }));
            }
            Some("tool-return") => {
                out.tools.push(json!({
                    "type": "return",
                    "tool": part.get("tool_name").cloned().unwrap_or(Value::Null),
                    "content": part.get("content").cloned().unwrap_or(Value::Null),
                }));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Collects clip IDs from Producer tool return contents in `values`.
pub(crate) fn clip_ids_in(values: &[&Value]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |v: Option<&Value>| {
        if let Some(id) = v.and_then(Value::as_str).filter(|s| !s.is_empty())
            && !out.iter().any(|seen| seen == id)
        {
            out.push(id.to_string());
        }
    };
    for value in values {
        push(value.get("clip_id"));
        push(value.get("clip_id_b"));
        push(value.get("result_clip_id"));
        for key in ["clip_outputs", "stems"] {
            if let Some(list) = value.get(key).and_then(Value::as_array) {
                for item in list {
                    push(item.get("clip_id"));
                }
            }
        }
    }
    out
}

/// Fetches the clip records for `ids` from `POST /clips`.
pub(crate) async fn fetch_clips(client: &MusicClient, ids: &[String]) -> Result<Vec<Value>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let res = client.post("/clips", json!({ "clip_ids": ids })).await?;
    let map = res.get("clips").and_then(Value::as_object);
    Ok(ids
        .iter()
        .filter_map(|id| map.and_then(|m| m.get(id)).cloned())
        .filter(|clip| !clip.is_null())
        .collect())
}

/// Returns the status of the `duration` field of a clip record.
fn duration_status(clip: &Value) -> &str {
    clip.pointer("/duration/status")
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Polls the clip records for `ids` until every clip has completed audio, then returns them.
pub(crate) async fn wait_clips_ready(
    client: &MusicClient,
    ids: &[String],
    timeout: Duration,
    progress: &ProgressCallback,
) -> Result<Vec<Value>> {
    let deadline = Instant::now() + timeout;
    loop {
        let clips = fetch_clips(client, ids).await?;
        if let Some(bad) = clips
            .iter()
            .find(|c| matches!(duration_status(c), "failed" | "error"))
        {
            anyhow::bail!(
                "Clip {} failed to render",
                bad.get("id").and_then(Value::as_str).unwrap_or("?")
            );
        }
        let ready = clips.len() == ids.len()
            && clips.iter().all(|c| {
                duration_status(c) == "completed"
                    && c.get("audio_url").and_then(Value::as_str).is_some()
            });
        if ready {
            return Ok(clips);
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "Timed out waiting for clips {} to finish rendering",
                ids.join(", ")
            );
        }
        progress(JobEvent::Progress(format!(
            "rendering {} clip(s)",
            ids.len()
        )));
        tokio::time::sleep(CLIP_POLL_INTERVAL).await;
    }
}

/// Builds a compact clip summary for tool results.
pub(crate) fn clip_summary(clip: &Value) -> Value {
    let duration = clip.pointer("/duration/value").and_then(|v| {
        v.as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or(v.as_f64())
    });
    json!({
        "id": clip.get("id"),
        "title": clip.get("title"),
        "type": clip.get("op_type"),
        "duration_s": duration.map(|d| (d * 10.0).round() / 10.0),
        "status": duration_status(clip),
        "privacy": clip.get("privacy"),
        "lyrics": clip.pointer("/lyrics/value/text"),
        "sound_prompt": clip.pointer("/operation/sound_prompt"),
        "session_id": clip.pointer("/operation/conversation_id"),
        "audio_url": clip.get("audio_url"),
        "wav_url": clip.get("wav_url"),
        "image_url": clip.get("image_url"),
        "video_url": clip.get("video_url"),
        "created_at": clip.get("created_at"),
    })
}
