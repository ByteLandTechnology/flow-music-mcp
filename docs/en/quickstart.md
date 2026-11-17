# Quickstart

Follow this guide to install `flow-music-mcp`, connect an MCP client, sign in to Google Flow Music, and make a song.

## Prerequisites

- **Google Chrome or Chromium**: The server needs Chrome or Chromium at runtime. Set `--chrome <PATH>` or `FLOW_MUSIC_MCP_CHROME` if auto-detection does not find your browser.
- **Google account with Google Flow Music access**: Make sure that the account can open [Google Flow Music](https://www.flowmusic.app) and that it has credits.
- **Node.js `18` or newer**: Node.js runs `@flow-music-mcp/cli`. You can also build the binary from source.

## 1. Install

```sh
npx -y @flow-music-mcp/cli --help
```

Or build from source:

```sh
git clone https://github.com/ByteLandTechnology/flow-music-mcp.git
cd flow-music-mcp
npm ci
cargo build --release
./target/release/flow-music-mcp --help
```

## 2. Configure your MCP client

```json
{
  "mcpServers": {
    "flow-music": {
      "command": "npx",
      "args": ["-y", "@flow-music-mcp/cli", "serve", "--home", "/path/to/.flow-music-mcp"]
    }
  }
}
```

The `--home` directory holds the Chrome profile, the SQLite index, and all downloads. The default is `~/.flow-music-mcp`.

## 3. Sign in

Call `music_login` with `{"action": "login"}`. A visible Chrome window opens at Google Flow Music:

1. Select **Log in** and finish Google Sign-In.
2. Finish the age check and the welcome steps if the site shows them.
3. Close the window, or wait. The server detects the session, closes the window, and starts headless Chrome on the same profile.

Then call `music_status` with `{"probe": true}`. The result shows `signed_in: true`, your plan, and your credits.

You can also sign in from a terminal with `flow-music-mcp login --home /path/to/.flow-music-mcp`. Stop your MCP client first, because two `flow-music-mcp` processes cannot use one Chrome profile at the same time. Run `flow-music-mcp doctor` to check the Chrome binary, the data directory, and the sign-in status.

## 4. Make a song

Call `music_generate`:

```json
{
  "prompt": "warm lo-fi hip hop with soft piano and vinyl crackle",
  "instrumental": true,
  "title": "Night Study"
}
```

The tool returns a job with an eight-character `id`. Call `music_wait` with that `id`. One call waits for at most 45 seconds. If the result has `pending: true`, call `music_wait` again.

When the job is `done`, the result gives:

- `created`: the new clip IDs.
- `files`: the local audio files in `<home>/downloads`.
- `resources`: `music://downloads/<id>` URIs that the client can read.
- `result.clips`: title, duration, lyrics, and public audio URLs.

## 5. Continue

- Make lyrics with `music_lyrics`, then pass `lyrics_id` to `music_generate`.
- Extend a song with `music_edit` and `{"action": "extend", "clip_id": "...", "extend_s": 30}`.
- Split stems with `music_stems`.
- Download WAV or MP3 with `music_download` and `{"id": "...", "format": "wav"}`.
- Ask the Producer agent for anything else with `music_chat`.

Read [Tools Reference](tools-reference.md) for all parameters.
