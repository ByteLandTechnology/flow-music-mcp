// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Playlists and Spaces in the Google Flow Music library.

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::MusicClient;

/// Parameters for `music_playlists`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct PlaylistsParams {
    /// Action: "list" (default), "favorites", "get", "create", "update", "delete", "add", "remove", or "reorder".
    pub(crate) action: Option<String>,
    /// Playlist ID for every action except "list", "favorites", and "create".
    pub(crate) id: Option<String>,
    /// Playlist name for "create" and "update".
    pub(crate) name: Option<String>,
    /// Playlist description for "create" and "update".
    pub(crate) description: Option<String>,
    /// Clip IDs for "add" and "remove", or the full new order for "reorder".
    pub(crate) clip_ids: Option<Vec<String>>,
    /// Maximum number of playlists for "list" (default 20, max 100).
    pub(crate) limit: Option<u32>,
    /// Number of playlists to skip for "list" (default 0).
    pub(crate) offset: Option<u32>,
}

/// Parameters for `music_spaces`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SpacesParams {
    /// Action: "list" (default), "get", "privacy", or "delete". Create a Space with music_chat.
    pub(crate) action: Option<String>,
    /// Space version ID (the "id" field of "list") for "get", "privacy", and "delete".
    pub(crate) id: Option<String>,
    /// New privacy for "privacy": "public", "unlisted", or "private".
    pub(crate) privacy: Option<String>,
    /// Include the source files of each Space (default false).
    pub(crate) include_files: Option<bool>,
    /// Maximum number of Spaces for "list" (default 20, max 100).
    pub(crate) limit: Option<u32>,
    /// Number of Spaces to skip for "list" (default 0).
    pub(crate) offset: Option<u32>,
}

/// Returns the ID of the signed-in user.
async fn user_id(client: &MusicClient) -> Result<String> {
    let me = client.get("/users/me").await?;
    me.get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("GET /users/me returned no user ID"))
}

/// Returns the non-empty `id` field of a request, or an error.
fn required<'a>(value: &'a Option<String>, field: &str) -> Result<&'a str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("This action needs '{}'", field))
}

/// Returns the non-empty clip ID list of a request, or an error.
fn clip_ids(req: &PlaylistsParams) -> Result<Vec<String>> {
    let ids = req.clip_ids.clone().unwrap_or_default();
    anyhow::ensure!(!ids.is_empty(), "This action needs 'clip_ids'");
    Ok(ids)
}

/// Runs a `music_playlists` action.
pub(crate) async fn playlists(client: &MusicClient, req: PlaylistsParams) -> Result<Value> {
    let limit = req.limit.unwrap_or(20).clamp(1, 100);
    let offset = req.offset.unwrap_or(0);
    match req.action.as_deref().unwrap_or("list") {
        action @ ("list" | "favorites") => {
            let uid = user_id(client).await?;
            let path = format!(
                "/users/{uid}/playlists?favorites={}&public=false&offset={offset}&limit={limit}",
                action == "favorites"
            );
            client.get(&path).await
        }
        "get" => {
            let id = required(&req.id, "id")?;
            client
                .get(&format!("/playlists/{id}?offset=0&limit=200"))
                .await
        }
        "create" => {
            let name = required(&req.name, "name")?;
            client
                .post(
                    "/playlists",
                    json!({ "name": name, "image_id": null, "description": req.description }),
                )
                .await
        }
        "update" | "rename" => {
            let id = required(&req.id, "id")?;
            let mut body = serde_json::Map::new();
            if let Some(name) = req.name.as_deref().filter(|s| !s.trim().is_empty()) {
                body.insert("name".to_string(), json!(name));
            }
            if let Some(desc) = req.description.as_deref() {
                body.insert("description".to_string(), json!(desc));
            }
            anyhow::ensure!(
                !body.is_empty(),
                "Action \"update\" needs 'name' or 'description'"
            );
            client
                .api(
                    "PATCH",
                    &format!("/playlists/{id}"),
                    Some(Value::Object(body)),
                )
                .await
        }
        "delete" => {
            let id = required(&req.id, "id")?;
            client
                .api("DELETE", &format!("/playlists/{id}"), None)
                .await?;
            Ok(json!({ "deleted": id }))
        }
        "add" => {
            let id = required(&req.id, "id")?;
            let ids = clip_ids(&req)?;
            client
                .post(
                    "/playlists/add",
                    json!({ "playlist_id": id, "clip_ids": ids }),
                )
                .await?;
            Ok(json!({ "playlist_id": id, "added": ids }))
        }
        "remove" => {
            let id = required(&req.id, "id")?;
            let ids = clip_ids(&req)?;
            client
                .api(
                    "DELETE",
                    &format!("/playlists/{id}/songs"),
                    Some(json!({ "clip_ids": ids })),
                )
                .await?;
            Ok(json!({ "playlist_id": id, "removed": ids }))
        }
        "reorder" => {
            let id = required(&req.id, "id")?;
            let ids = clip_ids(&req)?;
            client
                .post(
                    "/playlists/clips/reorder",
                    json!({ "playlist_id": id, "clip_ids": ids }),
                )
                .await?;
            Ok(json!({ "playlist_id": id, "order": ids }))
        }
        other => anyhow::bail!("Unknown playlists action: {}", other),
    }
}

/// Builds a compact Space summary, with source files when `include_files` is true.
///
/// The summary `id` is the Space version ID, because the Space API paths take that ID.
fn space_summary(space: &Value, include_files: bool) -> Value {
    let files = space.get("files").and_then(Value::as_array);
    let mut out = json!({
        "id": space.get("version_id"),
        "space_id": space.get("id"),
        "name": space.get("name"),
        "type": space.get("type"),
        "version": space.get("version"),
        "privacy": space.get("privacy"),
        "file_names": files.map(|f| f.iter().filter_map(|x| x.get("filepath")).cloned().collect::<Vec<_>>()),
    });
    if include_files {
        out["files"] = files.cloned().map(Value::Array).unwrap_or(Value::Null);
    }
    out
}

/// Runs a `music_spaces` action.
pub(crate) async fn spaces(client: &MusicClient, req: SpacesParams) -> Result<Value> {
    let include_files = req.include_files.unwrap_or(false);
    match req.action.as_deref().unwrap_or("list") {
        "list" => {
            let limit = req.limit.unwrap_or(20).clamp(1, 100);
            let offset = req.offset.unwrap_or(0);
            let res = client
                .get(&format!("/spaces/get?offset={offset}&limit={limit}"))
                .await?;
            let list: Vec<Value> = res
                .get("data")
                .and_then(Value::as_array)
                .map(|l| l.iter().map(|s| space_summary(s, include_files)).collect())
                .unwrap_or_default();
            Ok(json!({ "count": list.len(), "offset": offset, "spaces": list }))
        }
        "get" => {
            let id = required(&req.id, "id")?;
            let res = client.get(&format!("/space/{id}")).await?;
            let space = res.get("data").unwrap_or(&res);
            Ok(space_summary(space, include_files))
        }
        "privacy" => {
            let id = required(&req.id, "id")?;
            let privacy = required(&req.privacy, "privacy")?;
            anyhow::ensure!(
                matches!(privacy, "public" | "unlisted" | "private"),
                "privacy must be public, unlisted, or private"
            );
            client
                .api(
                    "PATCH",
                    &format!("/space/{id}"),
                    Some(json!({ "privacy": privacy })),
                )
                .await?;
            let res = client.get(&format!("/space/{id}")).await?;
            Ok(space_summary(
                res.get("data").unwrap_or(&res),
                include_files,
            ))
        }
        "delete" => {
            let id = required(&req.id, "id")?;
            client.api("DELETE", &format!("/space/{id}"), None).await?;
            Ok(json!({ "deleted": id }))
        }
        other => anyhow::bail!("Unknown spaces action: {}", other),
    }
}
