// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Songs (clips) and sessions (conversations) in the Google Flow Music library.

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::MusicClient;
use crate::producer::{clip_summary, fetch_clips};

/// Maximum number of characters of one message text in a session transcript.
const TRANSCRIPT_TEXT_CHARS: usize = 2000;

/// Parameters for `music_songs`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SongsParams {
    /// Action: "list" (default), "favorites", "get", "rename", "privacy", or "delete".
    pub(crate) action: Option<String>,
    /// Clip IDs for "get" and "delete", or one clip ID for "rename" and "privacy".
    pub(crate) ids: Option<Vec<String>>,
    /// New title for "rename".
    pub(crate) title: Option<String>,
    /// New privacy for "privacy": "public", "unlisted", or "private".
    pub(crate) privacy: Option<String>,
    /// Include disliked songs for "list" (default false).
    pub(crate) include_disliked: Option<bool>,
    /// Maximum number of songs for "list" and "favorites" (default 20, max 100).
    pub(crate) limit: Option<u32>,
    /// Number of songs to skip for "list" and "favorites" (default 0).
    pub(crate) offset: Option<u32>,
}

/// Parameters for `music_sessions`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SessionsParams {
    /// Action: "list" (default), "get", "create", "rename", "stop", or "delete".
    pub(crate) action: Option<String>,
    /// Session ID for "get", "rename", "stop", and "delete".
    pub(crate) id: Option<String>,
    /// New title for "rename".
    pub(crate) title: Option<String>,
    /// Maximum number of sessions for "list" (default 30, max 100).
    pub(crate) limit: Option<u32>,
    /// Number of sessions to skip for "list" (default 0).
    pub(crate) offset: Option<u32>,
    /// Also delete the songs of the session for "delete" (default false).
    pub(crate) delete_songs: Option<bool>,
    /// Also delete the Spaces of the session for "delete" (default false).
    pub(crate) delete_spaces: Option<bool>,
    /// Also delete the music videos of the session for "delete" (default false).
    pub(crate) delete_videos: Option<bool>,
}

/// Returns the one clip ID of `ids`, or an error.
fn single_id(ids: &Option<Vec<String>>) -> Result<&str> {
    match ids.as_deref() {
        Some([one]) if !one.is_empty() => Ok(one.as_str()),
        _ => anyhow::bail!("This action needs exactly one clip ID in 'ids'"),
    }
}

/// Returns the clip list of a list response.
fn clip_list(res: &Value) -> Vec<Value> {
    res.get("clips")
        .and_then(Value::as_array)
        .map(|list| list.iter().map(clip_summary).collect())
        .unwrap_or_default()
}

/// Runs a `music_songs` action.
pub(crate) async fn songs(client: &MusicClient, req: SongsParams) -> Result<Value> {
    let limit = req.limit.unwrap_or(20).clamp(1, 100);
    let offset = req.offset.unwrap_or(0);
    match req.action.as_deref().unwrap_or("list") {
        "list" => {
            let path = format!(
                "/clips/auth-user?limit={limit}&offset={offset}&filter=generations&include_disliked={}",
                req.include_disliked.unwrap_or(false)
            );
            let res = client.get(&path).await?;
            let clips = clip_list(&res);
            Ok(json!({ "count": clips.len(), "offset": offset, "songs": clips }))
        }
        "favorites" => {
            let res = client
                .get(&format!("/clips/favorites?limit={limit}&offset={offset}"))
                .await?;
            let clips = clip_list(&res);
            Ok(json!({ "count": clips.len(), "offset": offset, "songs": clips }))
        }
        "get" => {
            let ids = req.ids.unwrap_or_default();
            anyhow::ensure!(!ids.is_empty(), "Action \"get\" needs 'ids'");
            let clips = fetch_clips(client, &ids).await?;
            let (found, missing): (Vec<&Value>, Vec<&Value>) = clips
                .iter()
                .partition(|c| c.get("created_at").is_some_and(|v| !v.is_null()));
            let missing: Vec<&Value> = missing.iter().filter_map(|c| c.get("id")).collect();
            Ok(json!({
                "songs": found.into_iter().map(clip_summary).collect::<Vec<_>>(),
                "missing": missing,
            }))
        }
        "rename" => {
            let id = single_id(&req.ids)?;
            let title = req
                .title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Action \"rename\" needs 'title'"))?;
            let res = client
                .api(
                    "PATCH",
                    &format!("/clips/{id}"),
                    Some(json!({ "title": title })),
                )
                .await?;
            Ok(json!({ "song": res.get("clip").map(clip_summary) }))
        }
        "privacy" => {
            let id = single_id(&req.ids)?;
            let privacy = req.privacy.as_deref().unwrap_or("");
            anyhow::ensure!(
                matches!(privacy, "public" | "unlisted" | "private"),
                "privacy must be public, unlisted, or private"
            );
            let res = client
                .api(
                    "PATCH",
                    &format!("/clips/{id}"),
                    Some(json!({ "privacy": privacy })),
                )
                .await?;
            Ok(json!({ "song": res.get("clip").map(clip_summary) }))
        }
        "delete" => {
            let ids = req.ids.unwrap_or_default();
            anyhow::ensure!(!ids.is_empty(), "Action \"delete\" needs 'ids'");
            client
                .post("/clips/delete", json!({ "clip_ids": ids }))
                .await?;
            Ok(json!({ "deleted": ids }))
        }
        other => anyhow::bail!("Unknown songs action: {}", other),
    }
}

/// Truncates `text` to the transcript limit.
fn short_text(text: &str) -> String {
    if text.chars().count() <= TRANSCRIPT_TEXT_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(TRANSCRIPT_TEXT_CHARS).collect();
    out.push('…');
    out
}

/// Converts the messages of a conversation into a compact transcript.
fn transcript(messages: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for message in messages {
        let Some(parts) = message.get("parts").and_then(Value::as_array) else {
            continue;
        };
        for part in parts {
            let entry = match part.get("part_kind").and_then(Value::as_str) {
                Some("user-prompt") => {
                    let text = part.get("content").and_then(Value::as_str).unwrap_or("");
                    if text.starts_with("<ui-hidden>") {
                        continue;
                    }
                    json!({ "role": "user", "text": short_text(text) })
                }
                Some("text") => json!({
                    "role": "producer",
                    "text": short_text(part.get("content").and_then(Value::as_str).unwrap_or("")),
                }),
                Some("tool-call") => json!({
                    "role": "tool_call",
                    "tool": part.get("tool_name"),
                    "args": part.get("args"),
                }),
                Some("tool-return") => json!({
                    "role": "tool_return",
                    "tool": part.get("tool_name"),
                    "content": part.get("content"),
                }),
                _ => continue,
            };
            out.push(entry);
        }
    }
    out
}

/// Builds a compact session summary.
fn session_summary(conv: &Value) -> Value {
    json!({
        "id": conv.get("id"),
        "title": conv.get("title"),
        "created_at": conv.get("created_at"),
        "last_message_at": conv.get("last_message_at"),
        "project_id": conv.get("project_id"),
    })
}

/// Runs a `music_sessions` action.
pub(crate) async fn sessions(client: &MusicClient, req: SessionsParams) -> Result<Value> {
    let id = || {
        req.id
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("This action needs 'id'"))
    };
    match req.action.as_deref().unwrap_or("list") {
        "list" => {
            let limit = req.limit.unwrap_or(30).clamp(1, 100);
            let offset = req.offset.unwrap_or(0);
            let res = client
                .get(&format!("/conversations?limit={limit}&offset={offset}"))
                .await?;
            let list: Vec<Value> = res
                .as_array()
                .map(|l| l.iter().map(session_summary).collect())
                .unwrap_or_default();
            Ok(json!({ "count": list.len(), "offset": offset, "sessions": list }))
        }
        "get" => {
            let conv = client.get(&format!("/conversations/{}", id()?)).await?;
            let messages = conv
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut summary = session_summary(&conv);
            summary["transcript"] = json!(transcript(&messages));
            Ok(summary)
        }
        "create" => {
            let res = client
                .post("/conversation/create", json!({ "project_id": null }))
                .await?;
            Ok(json!({ "id": res.get("conversation_id") }))
        }
        "rename" => {
            let title = req
                .title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Action \"rename\" needs 'title'"))?;
            client
                .api(
                    "PATCH",
                    &format!("/conversations/{}", id()?),
                    Some(json!({ "title": title })),
                )
                .await?;
            Ok(json!({ "id": id()?, "title": title }))
        }
        "stop" => {
            let res = client
                .post(&format!("/conversations/{}/stop", id()?), json!({}))
                .await?;
            Ok(json!({ "stopped": id()?, "response": res }))
        }
        "delete" => {
            let body = json!({
                "delete_clips": req.delete_songs.unwrap_or(false),
                "delete_spaces": req.delete_spaces.unwrap_or(false),
                "delete_music_videos": req.delete_videos.unwrap_or(false),
            });
            client
                .api(
                    "DELETE",
                    &format!("/conversations/{}", id()?),
                    Some(body.clone()),
                )
                .await?;
            Ok(json!({ "deleted": id()?, "options": body }))
        }
        other => anyhow::bail!("Unknown sessions action: {}", other),
    }
}
