// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Streamable HTTP transport for the MCP server, with optional bearer token authentication.
//!
//! Without a token, the server listens only on a loopback address and keeps the `Host` header
//! check of `rmcp` against DNS rebinding. With a token, each request must send
//! `Authorization: Bearer <token>`, and the server accepts any `Host` header, so that other
//! computers can connect.

use anyhow::{Context, Result};
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::mcp::MusicMcpServer;

/// URL path of the MCP endpoint.
const MCP_PATH: &str = "/mcp";

/// Settings for the Streamable HTTP transport.
#[derive(Debug, Clone)]
pub(crate) struct HttpOptions {
    /// Socket address to listen on.
    pub(crate) addr: SocketAddr,
    /// Bearer token that each request must send, or `None` for no authentication.
    pub(crate) token: Option<String>,
}

/// Returns `true` when `a` equals `b`, in a time that depends only on the lengths.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Rejects with HTTP 403 a request that does not send `Authorization: Bearer <token>` with the correct token.
async fn require_token(State(token): State<Arc<str>>, request: Request, next: Next) -> Response {
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|given| constant_time_eq(given.as_bytes(), token.as_bytes()));
    if authorized {
        next.run(request).await
    } else {
        // 403 without `WWW-Authenticate`: MCP clients start an OAuth flow on 401, which this server does not support.
        (
            StatusCode::FORBIDDEN,
            "Missing or wrong bearer token. Send Authorization: Bearer <token>.",
        )
            .into_response()
    }
}

/// Serves `server` over Streamable HTTP at `<addr>/mcp` until `shutdown` completes.
///
/// Fails when `options.addr` is not a loopback address and `options.token` is `None`.
pub(crate) async fn serve_http(
    server: MusicMcpServer,
    options: HttpOptions,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    if options.token.is_none() && !options.addr.ip().is_loopback() {
        anyhow::bail!(
            "Set --http-token or FLOW_MUSIC_MCP_HTTP_TOKEN to listen on {}, because it is not a loopback address",
            options.addr
        );
    }

    let mut config = StreamableHttpServerConfig::default();
    if options.token.is_some() {
        config = config.disable_allowed_hosts();
    }
    let cancel = config.cancellation_token.clone();
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let mut router = axum::Router::new().nest_service(MCP_PATH, service);
    if let Some(token) = options.token {
        router = router.layer(middleware::from_fn_with_state(
            Arc::<str>::from(token),
            require_token,
        ));
    }

    let listener = tokio::net::TcpListener::bind(options.addr)
        .await
        .with_context(|| format!("Failed to listen on {}", options.addr))?;
    let local = listener.local_addr()?;
    eprintln!("flow-music-mcp serves MCP over HTTP at http://{local}{MCP_PATH}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown.await;
            cancel.cancel();
        })
        .await?;
    Ok(())
}
