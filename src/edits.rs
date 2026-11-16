// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Extend, replace, cover, and stem separation jobs.
//!
//! An edit is a recipe of three nodes: the input clip, the edit node, and the output
//! node. The server stores the recipe, and `audio__render_edit` renders it.

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::client::MusicClient;
use crate::db::MusicDb;
use crate::generation::{TOOL_TIMEOUT, finish_clips};
use crate::producer::{ToolRequest, clip_ids_in, run_tool};
use crate::queue::{JobResult, ProgressCallback};

/// Default cover strength that the page uses.
const DEFAULT_COVER_STRENGTH: f64 = 0.5;

/// One time region of a clip in seconds.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct Region {
    /// Start time in seconds.
    pub(crate) start_s: f64,
    /// End time in seconds.
    pub(crate) end_s: f64,
}

/// Parameters for `music_edit`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct EditParams {
    /// Edit action: "extend", "replace", or "cover".
    pub(crate) action: String,
    /// Source clip ID.
    pub(crate) clip_id: String,
    /// Instruction for the edit, for example "add a guitar solo". Default: a generic instruction for the action.
    pub(crate) instruction: Option<String>,
    /// Seconds of new audio for "extend" (default 30).
    pub(crate) extend_s: Option<f64>,
    /// Start point in seconds for "extend". Default: the end of the clip.
    pub(crate) extend_from_s: Option<f64>,
    /// Regions to rewrite for "replace".
    pub(crate) regions: Option<Vec<Region>>,
    /// Similarity to the source for "cover", from 0 to 1 (default 0.5).
    pub(crate) strength: Option<f64>,
    /// Seed for repeatable results.
    pub(crate) seed: Option<i64>,
    /// Title of the new clip. Default: the source title followed by the action in parentheses.
    pub(crate) title: Option<String>,
    /// Session ID for the edit. Default: the session of the source clip.
    pub(crate) session_id: Option<String>,
    /// Download the finished audio (default true).
    pub(crate) download: Option<bool>,
    /// Download format: "m4a" (default), "wav", or "mp3".
    pub(crate) format: Option<String>,
}

/// Parameters for `music_stems`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct StemsParams {
    /// Source clip ID.
    pub(crate) clip_id: String,
    /// Session ID for the job. Default: the session of the source clip.
    pub(crate) session_id: Option<String>,
    /// Download the four stems: vocals, drums, bass, and other (default true).
    pub(crate) download: Option<bool>,
    /// Download format: "m4a" (default), "wav", or "mp3".
    pub(crate) format: Option<String>,
}

/// Builds the edit node of the recipe for `req`.
fn edit_node(req: &EditParams) -> Result<Value> {
    let (node_type, default_instruction) = match req.action.as_str() {
        "extend" => ("ExtendSection", "Extend audio"),
        "replace" => ("ReplaceSection", "Replace section"),
        "cover" => ("CoverSong", "Cover song"),
        other => anyhow::bail!(
            "Unknown edit action '{}'. Use extend, replace, or cover.",
            other
        ),
    };
    let instruction = req
        .instruction
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_instruction);
    let mut node = Map::new();
    node.insert("node_type".to_string(), json!(node_type));
    node.insert("input_ids".to_string(), json!(["input"]));
    node.insert("output_ids".to_string(), json!(["processing"]));
    node.insert("instruction".to_string(), json!(instruction));
    node.insert("seed".to_string(), json!(req.seed));
    match req.action.as_str() {
        "extend" => {
            let extend_s = req.extend_s.unwrap_or(30.0);
            anyhow::ensure!(extend_s > 0.0, "extend_s must be greater than 0");
            node.insert("extend_s".to_string(), json!(extend_s));
            node.insert("extend_from_s".to_string(), json!(req.extend_from_s));
        }
        "replace" => {
            let regions = req.regions.clone().unwrap_or_default();
            anyhow::ensure!(
                !regions.is_empty(),
                "Action \"replace\" needs at least one region"
            );
            for r in &regions {
                anyhow::ensure!(
                    r.end_s > r.start_s && r.start_s >= 0.0,
                    "Each region needs 0 <= start_s < end_s"
                );
            }
            node.insert("masks".to_string(), json!(regions));
        }
        _ => {
            let strength = req.strength.unwrap_or(DEFAULT_COVER_STRENGTH);
            anyhow::ensure!(
                (0.0..=1.0).contains(&strength),
                "strength must be between 0 and 1"
            );
            node.insert("strength".to_string(), json!(strength));
        }
    }
    Ok(Value::Object(node))
}

/// Title and session ID of a source clip.
struct SourceInfo {
    /// Title of the source clip, or its ID.
    title: String,
    /// Session (conversation) ID of the source clip, if known.
    session_id: Option<String>,
}

/// Reads the title and session ID of the source clip `clip_id`.
async fn source_info(client: &MusicClient, clip_id: &str) -> SourceInfo {
    let clips = crate::producer::fetch_clips(client, &[clip_id.to_string()])
        .await
        .unwrap_or_default();
    let clip = clips.first();
    SourceInfo {
        title: clip
            .and_then(|c| c.get("title"))
            .and_then(Value::as_str)
            .unwrap_or(clip_id)
            .to_string(),
        session_id: clip
            .and_then(|c| c.pointer("/operation/conversation_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// Runs a `music_edit` job.
pub(crate) async fn run_edit(
    client: &MusicClient,
    db: &MusicDb,
    req: EditParams,
    progress: ProgressCallback,
) -> Result<JobResult> {
    let node = edit_node(&req)?;
    let source = source_info(client, &req.clip_id).await;
    let session_id = req.session_id.clone().or(source.session_id);
    let title = match req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        Some(t) => t.to_string(),
        None => format!("{} ({})", source.title, req.action),
    };
    let instruction = node.get("instruction").cloned().unwrap_or(Value::Null);
    let recipe = json!({
        "nodes": [
            { "node_type": "Input", "clip_id": req.clip_id, "input_ids": [], "output_ids": ["input"] },
            node,
            { "node_type": "Output", "input_ids": ["processing"], "output_ids": ["output"], "title": title, "generate_image": true },
        ],
        "instruction": instruction,
    });
    let created = client
        .post("/recipes/create", json!({ "recipe": recipe }))
        .await?;
    let recipe_id = created
        .get("recipe_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Recipe creation returned no recipe_id: {}", created))?
        .to_string();
    let outcome = run_tool(
        client,
        ToolRequest {
            tool_name: "audio__render_edit",
            args: json!({ "recipe_id": recipe_id, "title": title, "image_id": null }),
            conversation_id: session_id.as_deref(),
            lyrics_map: Map::new(),
            timeout: TOOL_TIMEOUT,
        },
        &progress,
    )
    .await?;
    finish_clips(
        client,
        db,
        clip_ids_in(&[&outcome.content]),
        req.download.unwrap_or(true),
        req.format.as_deref(),
        json!({ "session_id": outcome.conversation_id, "recipe_id": recipe_id, "source_clip_id": req.clip_id }),
        &progress,
    )
    .await
}

/// Runs a `music_stems` job.
pub(crate) async fn run_stems(
    client: &MusicClient,
    db: &MusicDb,
    req: StemsParams,
    progress: ProgressCallback,
) -> Result<JobResult> {
    let session_id = match req.session_id.clone() {
        Some(id) => Some(id),
        None => source_info(client, &req.clip_id).await.session_id,
    };
    let outcome = run_tool(
        client,
        ToolRequest {
            tool_name: "audio__split_stems",
            args: json!({ "clip_id": req.clip_id }),
            conversation_id: session_id.as_deref(),
            lyrics_map: Map::new(),
            timeout: TOOL_TIMEOUT,
        },
        &progress,
    )
    .await?;
    let stems: Vec<Value> = outcome
        .content
        .get("stems")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    finish_clips(
        client,
        db,
        clip_ids_in(&[&outcome.content]),
        req.download.unwrap_or(true),
        req.format.as_deref(),
        json!({ "session_id": outcome.conversation_id, "stems": stems, "source_clip_id": req.clip_id }),
        &progress,
    )
    .await
}
