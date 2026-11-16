# flow-music-mcp

[English](README.md) | [简体中文](docs/zh-CN/README.md) | [日本語](docs/ja/README.md)

`flow-music-mcp` is a single-binary Rust stdio Model Context Protocol (MCP) server for [Google Flow Music](https://www.flowmusic.app), the AI music studio from Google Labs (Lyria). It serves 18 MCP tools and download resources through `rmcp` over standard input and output.

`ChromeEngine` keeps a persistent Chrome or Chromium profile in `<home>/chrome-profile` (`0700` permissions on Unix). During background work, it connects over local Chrome DevTools Protocol (CDP) to headless Chrome (`--headless=new`) and keeps one page on `https://www.flowmusic.app`. An embedded TypeScript runtime sends the internal API requests from that page with the session of the page. During Google Sign-In in `music_login`, the server opens a visible Chrome window without CDP flags.

A single-worker `JobQueue` runs generation, lyrics, edit, stem, video, and chat jobs one at a time. The queue never retries a job automatically, because jobs spend credits. Finished audio goes into `<home>/downloads` with natural file names, and a SQLite index records each file.

## Quick install

Install Node.js `18` or newer and Google Chrome or Chromium on your host computer:

```sh
npx -y @flow-music-mcp/cli --help
```

`npm` installs the matching optional package (`@flow-music-mcp/darwin-arm64`, `@flow-music-mcp/darwin-x64`, `@flow-music-mcp/linux-arm64`, `@flow-music-mcp/linux-x64`, `@flow-music-mcp/win32-arm64`, or `@flow-music-mcp/win32-x64`). Do not pass `--omit=optional` or `--no-optional`.

To build from source, install Rust `1.88` (or newer) and Node.js. `esbuild` bundles `ts/index.ts` at compile time, so the binary does not need Node.js at runtime:

```sh
git clone https://github.com/ByteLandTechnology/flow-music-mcp.git
cd flow-music-mcp
npm ci
cargo build --release
```

Add the server to your MCP stdio client configuration (`mcpServers`):

```json
{
  "mcpServers": {
    "flow-music": {
      "command": "npx",
      "args": ["-y", "@flow-music-mcp/cli", "--home", "/path/to/.flow-music-mcp"]
    }
  }
}
```

## Documentation map

| Guide | Focus | Key topics |
| :--- | :--- | :--- |
| [Quickstart](docs/en/quickstart.md) | Tutorial | Install, configure a client, sign in, and make the first song |
| [Architecture](docs/en/architecture.md) | Explanation | `ChromeEngine`, page runtime, Producer tool calls, `JobQueue`, and storage |
| [Tools Reference](docs/en/tools-reference.md) | Reference | CLI options, environment variables, MCP resources, and all MCP tools |

## Tool overview

| Domain | Tools | Purpose |
| :--- | :--- | :--- |
| Session | `music_status`, `music_login` | Check the session, credits, and plan, and sign in with Chrome |
| Creation (queued) | `music_generate`, `music_lyrics`, `music_edit`, `music_stems`, `music_chat`, `music_video` | Make songs and lyrics, extend, replace, or cover songs, split stems, talk to the Producer agent, and make videos |
| Queue control | `music_wait`, `music_cancel`, `music_retry` | Wait for jobs (45 s per call), stop jobs, or run failed jobs again |
| Library | `music_songs`, `music_sessions`, `music_playlists`, `music_spaces`, `music_upload` | Manage songs, sessions, playlists, and Spaces, and upload audio or images |
| Files | `music_download`, `music_downloads` | Download audio (m4a, wav, mp3), video, or cover art, and manage local files |

> [!IMPORTANT]
> `flow-music-mcp` uses internal Google Flow Music interfaces. These interfaces can change without notice. Generation tools spend credits of your Google Flow Music plan.

> [!NOTE]
> Project status: the npm packages and GitHub releases are not published yet, so build from source for now.
