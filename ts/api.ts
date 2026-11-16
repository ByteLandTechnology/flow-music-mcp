// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

// Sends Google Flow Music internal API requests with the session of the page.

import { isExpired, readSession } from "./session";

/** Base path of the internal API on the application origin. */
const API_BASE = "/__api";

/** Maximum number of characters of a non-JSON response body that the runtime returns. */
const TEXT_LIMIT = 4000;

/** Result of one API request. */
export interface ApiResult {
  /** HTTP status code, or 0 when the runtime did not send the request. */
  status: number;
  /** Parsed JSON body, or truncated text when the body is not JSON. */
  body: unknown;
  /** Runtime error code: "signed_out" or "token_expired" when no request was sent. */
  error?: string;
}

/** One server-sent event from a job stream. */
export interface StreamEvent {
  /** Event identifier that a later read can pass as `last_id`. */
  id: string;
  /** Event name, for example "message", "part", "final", or "error". */
  event: string;
  /** Parsed JSON data of the event. */
  data: unknown;
}

/** Result of one bounded read of a job stream. */
export interface StreamResult {
  /** HTTP status code of the stream request. */
  status: number;
  /** Runtime error code when no request was sent. */
  error?: string;
  /** Events received during this read. */
  events: StreamEvent[];
  /** Identifier of the last received event, or the input identifier. */
  lastId: string;
  /** True when the stream sent "final" or "error", or closed. */
  done: boolean;
}

/** Binary response encoded for transfer over CDP. */
export interface BinaryResult {
  /** HTTP status code. */
  status: number;
  /** Runtime error code when no request was sent. */
  error?: string;
  /** Value of the `Content-Type` response header. */
  contentType: string;
  /** Value of the `Content-Disposition` response header. */
  contentDisposition: string;
  /** Base64 response bytes. */
  base64: string;
}

/** Returns the authorization header, or an error code when the session is not usable. */
function authHeader(): { header?: string; error?: string } {
  const session = readSession();
  if (!session) {
    return { error: "signed_out" };
  }
  if (isExpired(session)) {
    return { error: "token_expired" };
  }
  return { header: `Bearer ${session.accessToken}` };
}

/** Parses a response body as JSON, or returns truncated text. */
async function readBody(resp: Response): Promise<unknown> {
  const text = await resp.text();
  try {
    return JSON.parse(text);
  } catch {
    return text.slice(0, TEXT_LIMIT);
  }
}

/** Sends `method` to `path` below the API base with an optional JSON body. */
export async function api(method: string, path: string, body?: unknown): Promise<ApiResult> {
  const auth = authHeader();
  if (!auth.header) {
    return { status: 0, body: null, error: auth.error };
  }
  const headers: Record<string, string> = { Authorization: auth.header };
  if (body !== undefined && body !== null) {
    headers["content-type"] = "application/json";
  }
  const resp = await fetch(API_BASE + path, {
    method,
    headers,
    body: body === undefined || body === null ? undefined : JSON.stringify(body),
  });
  return { status: resp.status, body: await readBody(resp) };
}

/** Parses one server-sent event block into a stream event. */
function parseEventBlock(block: string): StreamEvent | null {
  let id = "";
  let event = "message";
  const data: string[] = [];
  for (const line of block.split(/\r?\n/)) {
    if (line.startsWith("id:")) {
      id = line.slice(3).trim();
    } else if (line.startsWith("event:")) {
      event = line.slice(6).trim();
    } else if (line.startsWith("data:")) {
      data.push(line.slice(5).trimStart());
    }
  }
  if (data.length === 0) {
    return null;
  }
  const raw = data.join("\n");
  try {
    return { id, event, data: JSON.parse(raw) };
  } catch {
    return { id, event, data: raw };
  }
}

/** Reads the stream of job `jobId` after `lastId` for at most `maxMs` milliseconds. */
export async function readStream(
  jobId: string,
  lastId: string,
  maxMs: number,
  toolProgress: boolean,
): Promise<StreamResult> {
  const auth = authHeader();
  if (!auth.header) {
    return { status: 0, error: auth.error, events: [], lastId, done: false };
  }
  const query = new URLSearchParams({ last_id: lastId || "0" });
  if (toolProgress) {
    query.set("tool_progress", "true");
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), maxMs);
  const events: StreamEvent[] = [];
  let cursor = lastId;
  let done = false;
  let status = 0;
  try {
    const resp = await fetch(`${API_BASE}/messages/${encodeURIComponent(jobId)}/stream?${query}`, {
      headers: { Authorization: auth.header },
      signal: controller.signal,
    });
    status = resp.status;
    if (!resp.ok || !resp.body) {
      return { status, events, lastId: cursor, done: true };
    }
    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    for (;;) {
      const chunk = await reader.read();
      if (chunk.value) {
        buffer += decoder.decode(chunk.value, { stream: !chunk.done });
      }
      let split = buffer.search(/\r?\n\r?\n/);
      while (split >= 0) {
        const block = buffer.slice(0, split);
        buffer = buffer.slice(split).replace(/^\r?\n\r?\n/, "");
        const parsed = parseEventBlock(block);
        if (parsed) {
          events.push(parsed);
          if (parsed.id) {
            cursor = parsed.id;
          }
          if (parsed.event === "final" || parsed.event === "error") {
            done = true;
          }
        }
        split = buffer.search(/\r?\n\r?\n/);
      }
      if (done || chunk.done) {
        done = true;
        await reader.cancel().catch(() => undefined);
        break;
      }
    }
  } catch (err) {
    if (!(err instanceof DOMException && err.name === "AbortError")) {
      throw err;
    }
  } finally {
    clearTimeout(timer);
  }
  return { status, events, lastId: cursor, done };
}

/** Decodes Base64 text to bytes. */
function fromBase64(b64: string): Uint8Array<ArrayBuffer> {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) {
    out[i] = bin.charCodeAt(i);
  }
  return out;
}

/** Encodes bytes to Base64 text in chunks to keep the call stack small. */
function toBase64(bytes: Uint8Array): string {
  let bin = "";
  const step = 0x8000;
  for (let i = 0; i < bytes.length; i += step) {
    bin += String.fromCharCode(...bytes.subarray(i, i + step));
  }
  return btoa(bin);
}

/** Uploads one file as multipart field `fieldName` to `path` with extra text `fields`. */
export async function upload(
  path: string,
  fieldName: string,
  fileName: string,
  mimeType: string,
  base64: string,
  fields: Record<string, string>,
): Promise<ApiResult> {
  const auth = authHeader();
  if (!auth.header) {
    return { status: 0, body: null, error: auth.error };
  }
  const form = new FormData();
  form.append(fieldName, new Blob([fromBase64(base64)], { type: mimeType }), fileName);
  for (const [key, value] of Object.entries(fields)) {
    form.append(key, value);
  }
  const resp = await fetch(API_BASE + path, {
    method: "POST",
    headers: { Authorization: auth.header },
    body: form,
  });
  return { status: resp.status, body: await readBody(resp) };
}

/** Downloads the API resource at `path` and returns its bytes as Base64. */
export async function fetchBinary(path: string): Promise<BinaryResult> {
  const auth = authHeader();
  if (!auth.header) {
    return { status: 0, error: auth.error, contentType: "", contentDisposition: "", base64: "" };
  }
  const resp = await fetch(API_BASE + path, { headers: { Authorization: auth.header } });
  const bytes = new Uint8Array(await resp.arrayBuffer());
  return {
    status: resp.status,
    contentType: resp.headers.get("content-type") ?? "",
    contentDisposition: resp.headers.get("content-disposition") ?? "",
    base64: resp.ok ? toBase64(bytes) : "",
  };
}
