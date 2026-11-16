// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Chrome DevTools Protocol (CDP) WebSocket client and local HTTP target discovery.

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;
use tokio::sync::{Mutex, oneshot};
use tokio_tungstenite::tungstenite::Message;

/// Error object returned inside a CDP JSON-RPC response frame.
#[derive(Debug, Deserialize)]
struct CdpError {
    /// Human-readable CDP failure description.
    message: Option<String>,
}

/// Incoming JSON frame received from Chrome over the CDP WebSocket.
#[derive(Debug, Deserialize)]
struct CdpInbound {
    /// Request identifier for command replies.
    id: Option<u64>,
    /// Result payload when a CDP command succeeds.
    result: Option<Value>,
    /// Error payload when a CDP command fails.
    error: Option<CdpError>,
    /// Event method name when the frame is an asynchronous CDP event.
    method: Option<String>,
}

/// Map of in-flight CDP command waiters and closed state.
struct PendingState {
    /// True once the WebSocket reader loop stops.
    closed: bool,
    /// Pending reply channels keyed by CDP command `id`.
    waiters: HashMap<u64, oneshot::Sender<Result<Value>>>,
}

/// Drop guard that removes a pending CDP waiter when a call cancels or times out.
struct PendingGuard<'a> {
    /// Shared pending waiter map.
    pending: &'a std::sync::Mutex<PendingState>,
    /// Command identifier owned by this call.
    id: u64,
    /// True while cleanup on drop remains active.
    armed: bool,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if self.armed
            && let Ok(mut guard) = self.pending.lock()
        {
            guard.waiters.remove(&self.id);
        }
    }
}

/// WebSocket sink type used to send CDP JSON commands.
type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

/// Shared WebSocket transport state for one CDP connection.
struct CdpTransport {
    /// Serialized outbound WebSocket sink.
    sink: Mutex<WsSink>,
    /// In-flight command response channels.
    pending: std::sync::Mutex<PendingState>,
    /// Monotonic CDP command ID generator.
    next_id: AtomicU64,
    /// True while the background WebSocket reader task is alive.
    alive: AtomicBool,
}

impl CdpTransport {
    /// Marks the transport closed and fails all pending command waiters.
    fn mark_closed(&self) {
        self.alive.store(false, Ordering::Release);
        if let Ok(mut guard) = self.pending.lock() {
            guard.closed = true;
            for (_, tx) in guard.waiters.drain() {
                let _ = tx.send(Err(anyhow::anyhow!("Chrome CDP connection closed")));
            }
        }
    }

    /// Sends one CDP command and waits up to `timeout` for its reply frame.
    async fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut guard = self
                .pending
                .lock()
                .map_err(|_| anyhow::anyhow!("CDP waiter state lock poisoned"))?;
            if guard.closed {
                anyhow::bail!("Chrome CDP connection closed");
            }
            guard.waiters.insert(id, tx);
        }

        let mut pending_guard = PendingGuard {
            pending: &self.pending,
            id,
            armed: true,
        };

        let payload = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        {
            let mut sink = self.sink.lock().await;
            sink.send(Message::Text(payload.to_string().into()))
                .await
                .with_context(|| format!("Failed to send CDP command {}", method))?;
        }

        let reply = tokio::time::timeout(timeout, rx)
            .await
            .with_context(|| format!("Timed out waiting for CDP command {}", method))?
            .with_context(|| format!("Chrome CDP closed during {}", method))?;
        pending_guard.armed = false;
        reply
    }
}

/// WebSocket CDP client for one Chrome target.
pub(crate) struct CdpConnection {
    /// Shared WebSocket transport.
    transport: Arc<CdpTransport>,
    /// Background task reading inbound CDP frames.
    reader_handle: tokio::task::JoinHandle<()>,
}

impl Drop for CdpConnection {
    fn drop(&mut self) {
        self.transport.mark_closed();
        self.reader_handle.abort();
    }
}

impl CdpConnection {
    /// Connects to `ws_url` and enables the `Page` and `Runtime` domains.
    pub(crate) async fn connect_page(ws_url: &str) -> Result<Self> {
        let conn = Self::connect_raw(ws_url).await?;
        conn.call("Page.enable", json!({})).await?;
        conn.call("Runtime.enable", json!({})).await?;
        Ok(conn)
    }

    /// Connects to `ws_url` without enabling page domains.
    pub(crate) async fn connect_basic(ws_url: &str) -> Result<Self> {
        Self::connect_raw(ws_url).await
    }

    /// Opens the WebSocket connection and spawns the inbound frame dispatcher task.
    async fn connect_raw(ws_url: &str) -> Result<Self> {
        let (ws_stream, _) = tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async(ws_url),
        )
        .await
        .with_context(|| format!("Timed out connecting to Chrome CDP WebSocket at {}", ws_url))?
        .with_context(|| format!("Failed to connect to Chrome CDP WebSocket at {}", ws_url))?;

        let (sink, mut stream) = ws_stream.split();
        let transport = Arc::new(CdpTransport {
            sink: Mutex::new(sink),
            pending: std::sync::Mutex::new(PendingState {
                closed: false,
                waiters: HashMap::new(),
            }),
            next_id: AtomicU64::new(1),
            alive: AtomicBool::new(true),
        });

        let weak_transport: Weak<CdpTransport> = Arc::downgrade(&transport);
        let reader_handle = tokio::spawn(async move {
            while let Some(msg_res) = stream.next().await {
                let Ok(msg) = msg_res else {
                    break;
                };
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Binary(b) => match String::from_utf8(b.to_vec()) {
                        Ok(s) => s,
                        Err(_) => continue,
                    },
                    Message::Close(_) => break,
                    _ => continue,
                };
                let Ok(frame) = serde_json::from_str::<CdpInbound>(&text) else {
                    continue;
                };

                if let Some(id) = frame.id {
                    if let Some(t) = weak_transport.upgrade() {
                        let waiter = t
                            .pending
                            .lock()
                            .ok()
                            .and_then(|mut g| g.waiters.remove(&id));
                        if let Some(tx) = waiter {
                            if let Some(err) = frame.error {
                                let msg = err.message.as_deref().unwrap_or("Chrome CDP error");
                                let _ = tx.send(Err(anyhow::anyhow!("Chrome CDP error: {}", msg)));
                            } else {
                                let _ = tx.send(Ok(frame.result.unwrap_or(Value::Null)));
                            }
                        }
                    }
                    continue;
                }

                let Some(method) = frame.method.as_deref() else {
                    continue;
                };
                if method == "Page.javascriptDialogOpening"
                    && let Some(t) = weak_transport.upgrade()
                {
                    tokio::spawn(async move {
                        let _ = t
                            .call(
                                "Page.handleJavaScriptDialog",
                                json!({ "accept": true }),
                                Duration::from_secs(5),
                            )
                            .await;
                    });
                }
            }

            if let Some(t) = weak_transport.upgrade() {
                t.mark_closed();
            }
        });

        Ok(Self {
            transport,
            reader_handle,
        })
    }

    /// Returns `true` when the WebSocket loop is still active.
    pub(crate) fn is_alive(&self) -> bool {
        self.transport.alive.load(Ordering::Acquire)
    }

    /// Sends one CDP command and awaits its result with a 30-second timeout.
    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.call_with_timeout(method, params, Duration::from_secs(30))
            .await
    }

    /// Sends one CDP command and awaits its result with a custom timeout.
    pub(crate) async fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        self.transport.call(method, params, timeout).await
    }
}

/// Returns the shared HTTP client for local Chrome CDP endpoint discovery.
fn http_client() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_millis(500))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Queries `/json/version` from a local Chrome CDP HTTP endpoint.
pub(crate) async fn fetch_json_version(endpoint: &str) -> Option<Value> {
    let url = format!("{}/json/version", endpoint.trim_end_matches('/'));
    let resp = http_client().get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Value>().await.ok()
}

/// Picks the preferred Google Flow Music or fallback `"page"` target from `/json/list`.
fn select_page_target(list: &[Value]) -> Option<&Value> {
    list.iter()
        .find(|t| {
            t.get("type").and_then(|v| v.as_str()) == Some("page")
                && t.get("url").and_then(|v| v.as_str()).is_some_and(|u| {
                    u.contains("flowmusic.app") || u.contains("accounts.google.com")
                })
        })
        .or_else(|| {
            list.iter()
                .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
        })
}

/// Finds or creates a page target on `endpoint` and returns its `webSocketDebuggerUrl`.
pub(crate) async fn find_or_create_page_target(endpoint: &str) -> Result<String> {
    let base = endpoint.trim_end_matches('/');
    let list_url = format!("{}/json/list", base);
    if let Ok(resp) = http_client().get(&list_url).send().await
        && resp.status().is_success()
        && let Ok(list) = resp.json::<Vec<Value>>().await
        && let Some(page) = select_page_target(&list)
        && let Some(ws) = page.get("webSocketDebuggerUrl").and_then(|v| v.as_str())
    {
        return Ok(ws.to_string());
    }

    let new_url = format!("{}/json/new?about:blank", base);
    let resp =
        http_client().put(&new_url).send().await.with_context(|| {
            format!("Failed to request a new Chrome page target at {}", new_url)
        })?;
    let page: Value = resp
        .json()
        .await
        .context("Failed to parse Chrome /json/new response")?;
    let ws = page
        .get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!("Missing webSocketDebuggerUrl in Chrome /json/new response")
        })?
        .to_string();
    Ok(ws)
}
