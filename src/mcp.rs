// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Model Context Protocol (MCP) stdio server adapter powered by `rmcp`.

use anyhow::Result;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, ContentBlock, Implementation, ListResourceTemplatesResult,
        ListResourcesResult, PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams,
        ReadResourceResponse, ReadResourceResult, ResourceTemplate, ServerCapabilities, ServerInfo,
    },
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
};
use serde::Serialize;
use serde_json::json;
use std::future::Future;
use std::sync::Arc;
use tokio::time::Duration;

use crate::auth::LoginParams;
use crate::chat::{ChatParams, VideoParams};
use crate::collections::{PlaylistsParams, SpacesParams};
use crate::db::{DOWNLOAD_ASSET_RESOURCE_PREFIX, DOWNLOAD_RESOURCE_PREFIX, DownloadsParams};
use crate::edits::{EditParams, StemsParams};
use crate::generation::{GenerateParams, LyricsParams};
use crate::library::{SessionsParams, SongsParams};
use crate::queue::{JobIdParams, WaitParams};
use crate::service::{self, DownloadParams, MusicService, ResourceReadError, StatusParams};
use crate::uploads::UploadParams;

/// Inline wait limit (`45_000` ms) before slow tools return a pending task handle.
const PATIENCE_MS: u64 = 45_000;

/// Maximum resource entries returned per `resources/list` page.
const RESOURCE_PAGE_SIZE: usize = 100;

/// MCP stdio server that routes tool calls to [`MusicService`].
#[derive(Clone)]
pub(crate) struct MusicMcpServer {
    /// Application service instance.
    service: Arc<MusicService>,
    /// Generated `rmcp` router for registered tools.
    tool_router: ToolRouter<Self>,
}

impl MusicMcpServer {
    /// Creates a [`MusicMcpServer`] that delegates tool calls to `service`.
    pub(crate) fn new(service: Arc<MusicService>) -> Self {
        Self {
            service,
            tool_router: Self::tool_router(),
        }
    }

    /// Serializes `value` as pretty JSON inside an MCP tool success result.
    fn ok_json<T: Serialize>(value: &T) -> CallToolResult {
        match serde_json::to_string_pretty(value) {
            Ok(body) => CallToolResult::success(vec![ContentBlock::text(body)]),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!(
                "Failed to serialize response JSON: {}",
                e
            ))]),
        }
    }

    /// Formats a fallible service result as an MCP tool success or error response.
    fn format_result<T: Serialize>(res: Result<T>) -> CallToolResult {
        match res {
            Ok(v) => Self::ok_json(&v),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e.to_string())]),
        }
    }

    /// Runs `fut` in a background task and waits up to 45 seconds before returning a pending payload.
    async fn run_patiently<F>(&self, tool: &str, fut: F) -> CallToolResult
    where
        F: Future<Output = Result<Vec<ContentBlock>>> + Send + 'static,
    {
        let task_id = format!("task_{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let (tx, rx) =
            tokio::sync::oneshot::channel::<std::result::Result<Vec<ContentBlock>, String>>();
        tokio::spawn(async move {
            let payload = fut.await.map_err(|e| e.to_string());
            let _ = tx.send(payload);
        });

        match tokio::time::timeout(Duration::from_millis(PATIENCE_MS), rx).await {
            Ok(Ok(Ok(blocks))) => CallToolResult::success(blocks),
            Ok(Ok(Err(e))) => CallToolResult::error(vec![ContentBlock::text(e)]),
            Ok(Err(_)) => CallToolResult::error(vec![ContentBlock::text(format!(
                "{} failed unexpectedly before returning a result",
                tool
            ))]),
            Err(_) => {
                let pending = json!({
                    "pending": true,
                    "task_id": task_id,
                    "message": format!("{} continues in the background. Check music_status or music_downloads later.", tool)
                });
                Self::ok_json(&pending)
            }
        }
    }

    /// Serves the MCP server over standard input and output until the client disconnects.
    pub(crate) async fn serve_stdio(self) -> Result<()> {
        self.serve(rmcp::transport::stdio())
            .await?
            .waiting()
            .await?;
        Ok(())
    }
}

#[tool_router]
impl MusicMcpServer {
    /// Reports session, account, credits, job queue, download index, and browser state.
    #[tool(
        description = "Report Google Flow Music session state, job queue, download index, and browser state. Set probe=true to start Chrome and read the account, plan, and credits."
    )]
    async fn music_status(&self, Parameters(req): Parameters<StatusParams>) -> CallToolResult {
        Self::format_result(self.service.status(req.probe.unwrap_or(false)).await)
    }

    /// Signs in through Google Chrome, reports the session, or signs out.
    #[tool(
        description = "Sign in to Google Flow Music through a visible Google Chrome window (action \"login\"), report the session (\"status\"), or delete the saved session (\"logout\")."
    )]
    async fn music_login(&self, Parameters(req): Parameters<LoginParams>) -> CallToolResult {
        Self::format_result(self.service.login(req).await)
    }

    /// Waits for queued or running jobs, returning early after 45 seconds if still pending.
    #[tool(description = "Wait for queued or running Google Flow Music jobs to finish.")]
    async fn music_wait(&self, Parameters(req): Parameters<WaitParams>) -> CallToolResult {
        match tokio::time::timeout(Duration::from_millis(PATIENCE_MS), self.service.wait(req)).await
        {
            Ok(res) => Self::format_result(res),
            Err(_) => {
                let jobs = self.service.list_jobs().await;
                let pending = json!({
                    "pending": true,
                    "jobs": jobs,
                    "message": "Jobs are still running in the background. Call music_wait again later."
                });
                Self::ok_json(&pending)
            }
        }
    }

    /// Cancels a queued or running job and stops its server-side session job.
    #[tool(
        description = "Cancel a queued or running Google Flow Music job. A queued job spends no credits. For a running job, the server keeps rendering the clips that a tool call already started, and spends their credits; the cancel stops the wait, the download, and the session."
    )]
    async fn music_cancel(&self, Parameters(req): Parameters<JobIdParams>) -> CallToolResult {
        Self::format_result(self.service.cancel(&req.id).await)
    }

    /// Runs a failed or cancelled job again.
    #[tool(
        description = "Run a failed or cancelled Google Flow Music job again. This spends credits again."
    )]
    async fn music_retry(&self, Parameters(req): Parameters<JobIdParams>) -> CallToolResult {
        Self::format_result(self.service.retry(&req.id).await)
    }

    /// Queues a song generation job.
    #[tool(
        description = "Queue a song generation job with Lyria (spends credits). Give a sound prompt, and optional lyrics, instrumental, BPM, length, title, seed, and prompt image. Lyrics, BPM, and length are hints. The job downloads the audio when it finishes. Use music_wait to get the result."
    )]
    async fn music_generate(&self, Parameters(req): Parameters<GenerateParams>) -> CallToolResult {
        Self::format_result(self.service.generate(req).await)
    }

    /// Queues a lyrics writing or lyrics edit job.
    #[tool(
        description = "Queue a lyrics job: write new lyrics from a prompt, or edit existing lyrics. The result gives the lyrics text, a title, and a matching sound prompt."
    )]
    async fn music_lyrics(&self, Parameters(req): Parameters<LyricsParams>) -> CallToolResult {
        Self::format_result(self.service.lyrics(req).await)
    }

    /// Queues an extend, replace, or cover edit of a song.
    #[tool(
        description = "Queue an edit of a song (spends credits): \"extend\" adds new audio, \"replace\" rewrites time regions, and \"cover\" restyles the song but keeps its melody. The result is a new clip."
    )]
    async fn music_edit(&self, Parameters(req): Parameters<EditParams>) -> CallToolResult {
        Self::format_result(self.service.edit(req).await)
    }

    /// Queues a stem separation job.
    #[tool(
        description = "Queue a stem separation of a song into vocals, drums, bass, and other. The job downloads the four stems."
    )]
    async fn music_stems(&self, Parameters(req): Parameters<StemsParams>) -> CallToolResult {
        Self::format_result(self.service.stems(req).await)
    }

    /// Queues a chat message to the Producer agent.
    #[tool(
        description = "Queue a message to the Google Flow Music Producer agent. The Producer can write lyrics, generate and edit songs, split stems, build Spaces, and plan music videos by itself, thus a message can spend credits. Use a session ID to continue a conversation."
    )]
    async fn music_chat(&self, Parameters(req): Parameters<ChatParams>) -> CallToolResult {
        Self::format_result(self.service.chat(req).await)
    }

    /// Lists, inspects, or deletes music videos, or queues a new video.
    #[tool(
        description = "List, get, check status of, or delete music videos. Action \"create\" queues a new video for a song. Kind \"social\" makes a short vertical 720p clip, waits for the render, and downloads the MP4; the server can make it without credits. Kind \"music\" asks the Producer for a full music video, spends credits, and is not yet verified against the live service."
    )]
    async fn music_video(&self, Parameters(req): Parameters<VideoParams>) -> CallToolResult {
        Self::format_result(self.service.video(req).await)
    }

    /// Lists, inspects, renames, changes privacy of, or deletes songs.
    #[tool(
        description = "List, get, rename, change privacy of, or delete songs (clips) in the Google Flow Music library."
    )]
    async fn music_songs(&self, Parameters(req): Parameters<SongsParams>) -> CallToolResult {
        Self::format_result(self.service.songs(req).await)
    }

    /// Lists, reads, creates, renames, stops, or deletes sessions.
    #[tool(
        description = "List, read the transcript of, create, rename, stop, or delete Google Flow Music sessions (Producer conversations)."
    )]
    async fn music_sessions(&self, Parameters(req): Parameters<SessionsParams>) -> CallToolResult {
        Self::format_result(self.service.sessions(req).await)
    }

    /// Manages playlists and their songs.
    #[tool(
        description = "List, get, create, update, or delete playlists, and add, remove, or reorder songs in a playlist."
    )]
    async fn music_playlists(
        &self,
        Parameters(req): Parameters<PlaylistsParams>,
    ) -> CallToolResult {
        Self::format_result(self.service.playlists(req).await)
    }

    /// Lists, inspects, changes the privacy of, or deletes Spaces.
    #[tool(
        description = "List, get, change the privacy of, or delete Spaces (interactive instruments and tools). To create a Space, describe it to the Producer with music_chat."
    )]
    async fn music_spaces(&self, Parameters(req): Parameters<SpacesParams>) -> CallToolResult {
        Self::format_result(self.service.spaces(req).await)
    }

    /// Uploads a local audio or image file.
    #[tool(
        description = "Upload a local audio file (becomes a clip that music_edit and music_stems can use) or an image file (gives an image ID for music_generate)."
    )]
    async fn music_upload(&self, Parameters(req): Parameters<UploadParams>) -> CallToolResult {
        let service = Arc::clone(&self.service);
        self.run_patiently("music_upload", async move {
            let res = service.upload(req).await?;
            Ok(vec![ContentBlock::text(serde_json::to_string_pretty(
                &res,
            )?)])
        })
        .await
    }

    /// Downloads the audio, video, or cover art of a song.
    #[tool(
        description = "Download the audio (m4a, wav, or mp3), video, or cover art of a song, or a music video, into the local downloads directory."
    )]
    async fn music_download(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(req): Parameters<DownloadParams>,
    ) -> CallToolResult {
        let service = Arc::clone(&self.service);
        let include_link = context
            .protocol_version()
            .is_none_or(|v| v.as_str() >= ProtocolVersion::V_2025_06_18.as_str());
        self.run_patiently("music_download", async move {
            let record = service.download(req).await?;
            let payload = json!({
                "file": record.file_path,
                "resource_uri": record.resource_uri,
                "record": record,
            });
            let text = serde_json::to_string_pretty(&payload)?;
            let mut blocks = vec![ContentBlock::text(text)];
            if include_link {
                blocks.push(ContentBlock::resource_link(
                    service::record_to_mcp_resource(&record),
                ));
            }
            Ok(blocks)
        })
        .await
    }

    /// Queries, inspects, reads chunks from, prunes, deletes, or clears local downloads.
    #[tool(
        description = "List, inspect, read binary chunks, prune, delete, or clear locally downloaded files."
    )]
    async fn music_downloads(
        &self,
        Parameters(req): Parameters<DownloadsParams>,
    ) -> CallToolResult {
        Self::format_result(self.service.downloads(req).await)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MusicMcpServer {
    /// Returns MCP server capabilities and version metadata.
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(
            "flow-music-mcp",
            env!("CARGO_PKG_VERSION"),
        ))
    }

    /// Lists downloaded media files as paginated MCP resources.
    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListResourcesResult, ErrorData> {
        let all = self
            .service
            .list_download_resources()
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let offset = match request.and_then(|p| p.cursor) {
            Some(c) => c.parse::<usize>().map_err(|_| {
                ErrorData::invalid_params("Invalid pagination cursor for resources/list", None)
            })?,
            None => 0,
        };
        if offset > all.len() || (offset == all.len() && offset > 0) {
            return Err(ErrorData::invalid_params(
                "Pagination cursor out of range for resources/list",
                None,
            ));
        }
        let end = (offset + RESOURCE_PAGE_SIZE).min(all.len());
        let page = all[offset..end].to_vec();
        let mut res = ListResourcesResult::with_all_items(page);
        if end < all.len() {
            res.next_cursor = Some(end.to_string());
        }
        Ok(res)
    }

    /// Advertises MCP resource URI templates for record IDs and Google Flow Music clip IDs.
    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListResourceTemplatesResult, ErrorData> {
        let by_id = ResourceTemplate::new(
            format!("{}{{id}}", DOWNLOAD_RESOURCE_PREFIX),
            "music-download-by-id",
        )
        .with_title("Downloaded Media by Record ID")
        .with_description("Read a downloaded media file by its SQLite row ID.");

        let by_asset = ResourceTemplate::new(
            format!("{}{{asset_id}}", DOWNLOAD_ASSET_RESOURCE_PREFIX),
            "music-download-by-asset-id",
        )
        .with_title("Downloaded File by Clip ID")
        .with_description(
            "Read the latest downloaded file for a Google Flow Music clip or video ID.",
        );

        Ok(ListResourceTemplatesResult::with_all_items(vec![
            by_id, by_asset,
        ]))
    }

    /// Reads one downloaded media file addressed by a `music://downloads/...` URI.
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ReadResourceResponse, ErrorData> {
        match self.service.read_download_resource(&request.uri).await {
            Ok(contents) => Ok(ReadResourceResult::new(vec![contents]).into()),
            Err(ResourceReadError::NotFound(msg)) => Err(ErrorData::resource_not_found(msg, None)),
            Err(ResourceReadError::InvalidUri(msg) | ResourceReadError::TooLarge(msg)) => {
                Err(ErrorData::invalid_params(msg, None))
            }
            Err(ResourceReadError::Internal(msg)) => Err(ErrorData::internal_error(msg, None)),
        }
    }
}
