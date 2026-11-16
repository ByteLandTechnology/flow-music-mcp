# Architecture

This guide explains how `flow-music-mcp` works.

## Components

| Component | Source | Role |
| :--- | :--- | :--- |
| MCP adapter | `src/mcp.rs` | Registers 18 tools and the `music://downloads/...` resources with `rmcp`. Slow tools return `pending: true` after 45 seconds. |
| Service | `src/service.rs` | Routes tools, queues jobs, and serves downloads and resources. |
| `ChromeEngine` | `src/engine.rs`, `src/chrome.rs`, `src/cdp.rs` | Starts Chrome, keeps one page on the application origin, and runs the page runtime. |
| Page runtime | `ts/*.ts` | Reads the session of the page and sends API requests, stream reads, uploads, and binary downloads. |
| API client | `src/client.rs` | Calls the page runtime and refreshes an expired session once by a page reload. |
| Producer layer | `src/producer.rs` | Sends Producer tool calls and chat messages, follows job streams, and waits for clips to render. |
| Domain modules | `src/generation.rs`, `src/edits.rs`, `src/chat.rs`, `src/library.rs`, `src/collections.rs`, `src/uploads.rs` | Build the requests for each tool. |
| `JobQueue` | `src/queue.rs` | Runs credit-spending jobs one at a time. |
| Storage | `src/db.rs`, `src/download.rs`, `src/media_files.rs` | Saves files in `<home>/downloads` and indexes them in SQLite. |

## Sign-in and session

Google Flow Music keeps its session in Supabase cookies (`sb-sb-auth-token.0`, `sb-sb-auth-token.1`, and more chunks) on `www.flowmusic.app`. The session lives only in the persistent Chrome profile at `<home>/chrome-profile`. The server does not copy session cookies to other files.

1. `music_login` opens a visible Chrome window without CDP flags on a clean profile.
2. The server reads scratch copies of the Chrome `Cookies` and `History` databases each second. It looks for the session cookie and for a Google Accounts rejection page.
3. When the session cookie exists, or when the user closes the window, the server stops the visible Chrome and starts headless Chrome on the same profile.
4. The page runtime confirms the session with `GET /__api/users/me`.

The access token expires after about one hour. When the runtime finds an expired token, or when the API returns HTTP 401, the client reloads the page. The page client then refreshes the token, and the client sends the request again once.

## Requests from the page

The internal API is at `https://www.flowmusic.app/__api/...`. Each request carries `Authorization: Bearer <access token>`. The runtime sends all requests with `fetch` from the page, thus the requests have the same origin, cookies, and browser identity as the website.

The Rust side uses only fixed API paths. No tool can send a request to a path that a caller selects.

## Producer tool calls

Song creation, edits, stems, lyrics, and videos are Producer tools:

1. `POST /__api/producer/tool-call` sends one tool call and returns a `job_id`.
2. `GET /__api/messages/<job_id>/stream` sends server-sent events. The runtime reads the stream in slices of 20 seconds and continues with `last_id`.
3. The `tool-return` part with the same `tool_call_id` carries the result, for example the new clip IDs.
4. A clip can still render after the tool return. The server polls `POST /__api/clips` until the audio of each clip is complete.

Edits use recipes. `POST /__api/recipes/create` stores three nodes: the input clip, an `ExtendSection`, `ReplaceSection`, or `CoverSong` node, and the output. Then the `audio__render_edit` tool renders the recipe.

`music_generate` builds the same arguments as the compose form of the website. The BPM goes into the sound prompt, and the length goes into the lyrics as an `[End - m:ss]` marker. The lyrics ID is the UUID version 5 of the lyrics text, and the lyrics text goes in the client context. The model uses lyrics, BPM, and length as guidance.

`music_chat` sends `POST /__api/conversation`. The Producer agent selects tools by itself, thus a chat message can spend credits.

## Job queue

`JobQueue` runs one job at a time on one worker task. Between jobs, the worker waits a random time from `FLOW_MUSIC_MCP_PAUSE_MIN_S` to `FLOW_MUSIC_MCP_PAUSE_MAX_S` (default 5 to 15 seconds).

- The queue never retries a job automatically. `music_retry` runs a failed or cancelled job again and spends credits again.
- `music_cancel` stops the local job. When the job has a session, the server also sends `POST /__api/conversations/<id>/stop`.
- The queue keeps the last 200 jobs in memory. A server restart clears the queue, but the songs stay in your library.

## Storage

All data is under `<home>` (default `~/.flow-music-mcp`):

| Path | Content | Unix permissions |
| :--- | :--- | :--- |
| `chrome-profile/` | Persistent Chrome profile with the session | `0700` |
| `flow-music-mcp.db` | SQLite index of downloads | `0600` |
| `downloads/` | Downloaded audio, video, and images | `0700` directory |

M4A, WAV, stem, and cover files come from public storage URLs. MP3 files come from `GET /__api/download/audio/<clip_id>?format=mp3` through the page. The server checks the leading bytes of each file before it indexes the file.
