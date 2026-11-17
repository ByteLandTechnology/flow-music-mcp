# flow-music-mcp

![English](https://img.shields.io/badge/English-2563eb?style=flat-square&logoWidth=16&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAxOCI%2BPGNsaXBQYXRoIGlkPSJ1Ij48cmVjdCB3aWR0aD0iMjQiIGhlaWdodD0iMTgiIHJ4PSIyIi8%2BPC9jbGlwUGF0aD48ZyBjbGlwLXBhdGg9InVybCgjdSkiPjxyZWN0IHdpZHRoPSIyNCIgaGVpZ2h0PSIxOCIgZmlsbD0iI2ZmZiIvPjxwYXRoIHN0cm9rZT0iI2IyMjIzNCIgc3Ryb2tlLXdpZHRoPSIxLjQiIGQ9Ik0wIC43aDI0TTAgMy41aDI0TTAgNi4zaDI0TTAgOS4xaDI0TTAgMTEuOWgyNE0wIDE0LjdoMjRNMCAxNy4zaDI0Ii8%2BPHJlY3Qgd2lkdGg9IjEwLjQiIGhlaWdodD0iOS44IiBmaWxsPSIjM2MzYjZlIi8%2BPHBhdGggc3Ryb2tlPSIjZmZmIiBzdHJva2Utd2lkdGg9IjEuMyIgc3Ryb2tlLWxpbmVjYXA9InJvdW5kIiBkPSJNMi4xIDIuMWguMDFNNS4yIDIuMWguMDFNOC4zIDIuMWguMDFNMy42IDQuOWguMDFNNi44IDQuOWguMDFNMi4xIDcuN2guMDFNNS4yIDcuN2guMDFNOC4zIDcuN2guMDEiLz48L2c%2BPC9zdmc%2B)
[![简体中文](https://img.shields.io/badge/%E7%AE%80%E4%BD%93%E4%B8%AD%E6%96%87-64748b?style=flat-square&logoWidth=16&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAxOCI%2BPHJlY3Qgd2lkdGg9IjI0IiBoZWlnaHQ9IjE4IiByeD0iMiIgZmlsbD0iI2RlMjkxMCIvPjxwYXRoIGZpbGw9IiNmZmRlMDAiIGQ9Ik00LjggMi40IDYuMyA3LjEgMi4zIDQuMiA3LjMgNC4yIDMuMyA3LjF6TTguNCAyLjYgOS41IDEuMyA5LjQgMy4wIDguNSAxLjUgMTAuMSAyLjF6TTEwLjAgNC4wIDExLjUgMy4xIDEwLjggNC43IDEwLjUgMy4wIDExLjcgNC4yek0xMC4wIDYuMiAxMS43IDYuMCAxMC40IDcuMiAxMC44IDUuNSAxMS41IDcuMXpNOC41IDcuNiAxMC4xIDguMSA4LjUgOC42IDkuNSA3LjIgOS41IDkuMHoiLz48L3N2Zz4%3D)](docs/zh-CN/README.md)
[![日本語](https://img.shields.io/badge/%E6%97%A5%E6%9C%AC%E8%AA%9E-64748b?style=flat-square&logoWidth=16&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAxOCI%2BPHJlY3Qgd2lkdGg9IjI0IiBoZWlnaHQ9IjE4IiByeD0iMiIgZmlsbD0iI2ZmZiIvPjxjaXJjbGUgY3g9IjEyIiBjeT0iOSIgcj0iNSIgZmlsbD0iI2JjMDAyZCIvPjwvc3ZnPg%3D%3D)](docs/ja/README.md)

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
