// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Typed access to the Google Flow Music internal API through the page runtime.
//!
//! Every request runs inside the headless page with the session of the page. When the
//! access token expired, the client reloads the page once so that the page client
//! refreshes the token, and then sends the request again.

use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

use crate::engine::ChromeEngine;

/// Error text for requests without a Google Flow Music session.
const NOT_SIGNED_IN: &str =
    "Not signed in to Google Flow Music. Run music_login with action \"login\" first.";

/// Maximum number of characters of an error body in an error message.
const ERROR_EXCERPT_CHARS: usize = 600;

/// One server-sent event from a job stream.
#[derive(Debug, Clone)]
pub(crate) struct StreamEvent {
    /// Event name, for example "message", "part", "final", or "error".
    pub(crate) event: String,
    /// Parsed JSON data of the event.
    pub(crate) data: Value,
}

/// Result of one bounded read of a job stream.
#[derive(Debug, Clone)]
pub(crate) struct StreamChunk {
    /// Events received during this read.
    pub(crate) events: Vec<StreamEvent>,
    /// Identifier of the last received event, for the next read.
    pub(crate) last_id: String,
    /// True when the stream finished.
    pub(crate) done: bool,
}

/// Binary response body with its headers.
#[derive(Debug, Clone)]
pub(crate) struct BinaryResponse {
    /// Value of the `Content-Type` header.
    pub(crate) content_type: Option<String>,
    /// Value of the `Content-Disposition` header.
    pub(crate) disposition: Option<String>,
    /// Response bytes.
    pub(crate) bytes: Vec<u8>,
}

/// Client for the Google Flow Music internal API.
#[derive(Clone)]
pub(crate) struct MusicClient {
    /// Browser engine that runs the page runtime.
    engine: Arc<ChromeEngine>,
}

/// Rejects an API path that could leave its fixed route.
///
/// Tools put caller IDs into paths. The path part may hold only ASCII letters, digits,
/// `-`, `_`, and `/`, with no empty segment. The query part may hold only ASCII letters,
/// digits, `-`, `_`, `.`, `=`, and `&`. Thus `..`, `#`, `%`, and a second `?` never reach
/// the browser, and an ID cannot select another endpoint.
fn check_api_path(path: &str) -> Result<()> {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let route_ok = route.starts_with('/')
        && route[1..].split('/').all(|seg| {
            !seg.is_empty()
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        });
    let query_ok = query
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'=' | b'&'));
    anyhow::ensure!(
        route_ok && query_ok,
        "Invalid ID or API path '{}'. IDs may hold only letters, digits, '-', and '_'.",
        path
    );
    Ok(())
}

/// Returns a non-empty string field of `value`.
fn str_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Formats a short error text from an API error body.
fn error_excerpt(body: &Value) -> String {
    let text = body
        .get("message")
        .or_else(|| body.get("detail"))
        .map(|v| match v.as_str() {
            Some(s) => s.to_string(),
            None => v.to_string(),
        })
        .unwrap_or_else(|| match body.as_str() {
            Some(s) => s.to_string(),
            None => body.to_string(),
        });
    text.chars().take(ERROR_EXCERPT_CHARS).collect()
}

impl MusicClient {
    /// Creates a client that uses `engine`.
    pub(crate) fn new(engine: Arc<ChromeEngine>) -> Self {
        Self { engine }
    }

    /// Returns the browser engine of this client.
    pub(crate) fn engine(&self) -> &Arc<ChromeEngine> {
        &self.engine
    }

    /// Calls runtime `method` and retries once after a page reload when the token expired.
    async fn call(&self, method: &str, args: Value, timeout: Duration) -> Result<Value> {
        for attempt in 0..2 {
            self.engine.ensure_app_page().await?;
            let res = self
                .engine
                .call_runtime(method, args.clone(), timeout)
                .await?;
            let status = res.get("status").and_then(Value::as_u64).unwrap_or(0);
            let error = res.get("error").and_then(Value::as_str).unwrap_or("");
            if error == "signed_out" {
                anyhow::bail!(NOT_SIGNED_IN);
            }
            let expired = error == "token_expired" || status == 401;
            if expired && attempt == 0 {
                self.engine.reload_app_page().await?;
                continue;
            }
            if expired {
                anyhow::bail!(NOT_SIGNED_IN);
            }
            return Ok(res);
        }
        anyhow::bail!(NOT_SIGNED_IN)
    }

    /// Sends `method` to API `path` and returns the JSON body of a successful response.
    pub(crate) async fn api(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        check_api_path(path)?;
        let args = json!([method, path, body.unwrap_or(Value::Null)]);
        let res = self.call("api", args, Duration::from_secs(60)).await?;
        let status = res.get("status").and_then(Value::as_u64).unwrap_or(0);
        let body = res.get("body").cloned().unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            anyhow::bail!(
                "Google Flow Music API {} {} returned HTTP {}: {}",
                method,
                path,
                status,
                error_excerpt(&body)
            );
        }
        Ok(body)
    }

    /// Sends a GET request to API `path`.
    pub(crate) async fn get(&self, path: &str) -> Result<Value> {
        self.api("GET", path, None).await
    }

    /// Sends a POST request with a JSON body to API `path`.
    pub(crate) async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.api("POST", path, Some(body)).await
    }

    /// Reads the stream of job `job_id` after `last_id` for at most `max_ms` milliseconds.
    pub(crate) async fn read_stream(
        &self,
        job_id: &str,
        last_id: &str,
        max_ms: u64,
        tool_progress: bool,
    ) -> Result<StreamChunk> {
        check_api_path(&format!("/messages/{job_id}/stream"))?;
        let args = json!([job_id, last_id, max_ms, tool_progress]);
        let timeout = Duration::from_millis(max_ms + 30_000);
        let res = self.call("readStream", args, timeout).await?;
        let status = res.get("status").and_then(Value::as_u64).unwrap_or(0);
        if status >= 400 {
            anyhow::bail!("Job stream {} returned HTTP {}", job_id, status);
        }
        let events = res
            .get("events")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .map(|e| StreamEvent {
                        event: str_field(e, "event").unwrap_or_else(|| "message".to_string()),
                        data: e.get("data").cloned().unwrap_or(Value::Null),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(StreamChunk {
            events,
            last_id: str_field(&res, "lastId").unwrap_or_else(|| last_id.to_string()),
            done: res.get("done").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    /// Uploads `bytes` as multipart field `field` to API `path` with extra text `fields`.
    pub(crate) async fn upload(
        &self,
        path: &str,
        field: &str,
        file_name: &str,
        mime_type: &str,
        bytes: &[u8],
        fields: Value,
    ) -> Result<Value> {
        check_api_path(path)?;
        let args = json!([
            path,
            field,
            file_name,
            mime_type,
            BASE64.encode(bytes),
            fields
        ]);
        let res = self.call("upload", args, Duration::from_secs(300)).await?;
        let status = res.get("status").and_then(Value::as_u64).unwrap_or(0);
        let body = res.get("body").cloned().unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            anyhow::bail!(
                "Upload to {} returned HTTP {}: {}",
                path,
                status,
                error_excerpt(&body)
            );
        }
        Ok(body)
    }

    /// Downloads the API resource at `path`.
    pub(crate) async fn fetch_binary(&self, path: &str) -> Result<BinaryResponse> {
        check_api_path(path)?;
        let res = self
            .call("fetchBinary", json!([path]), Duration::from_secs(300))
            .await?;
        let status = res.get("status").and_then(Value::as_u64).unwrap_or(0);
        if !(200..300).contains(&status) {
            anyhow::bail!("Download from {} returned HTTP {}", path, status);
        }
        let bytes = BASE64.decode(res.get("base64").and_then(Value::as_str).unwrap_or(""))?;
        Ok(BinaryResponse {
            content_type: str_field(&res, "contentType"),
            disposition: str_field(&res, "contentDisposition"),
            bytes,
        })
    }

    /// Computes the UUID version 5 of `name` in `namespace` with the page runtime.
    pub(crate) async fn uuid_v5(&self, name: &str, namespace: &str) -> Result<String> {
        self.engine.ensure_app_page().await?;
        let res = self
            .engine
            .call_runtime("uuidV5", json!([name, namespace]), Duration::from_secs(10))
            .await?;
        res.as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("The runtime did not return a UUID"))
    }
}
