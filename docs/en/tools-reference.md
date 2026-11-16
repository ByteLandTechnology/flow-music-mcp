# Tools Reference

This reference lists the command-line options, environment variables, MCP resources, and MCP tools of `flow-music-mcp`.

## Command-line options

| Option | Description |
| :--- | :--- |
| `--home <PATH>` | Data root directory. Default: `FLOW_MUSIC_MCP_HOME`, then `~/.flow-music-mcp`. |
| `--chrome <PATH>` | Chrome or Chromium binary. Default: `FLOW_MUSIC_MCP_CHROME`, then auto-detection. |

## Environment variables

| Variable | Description |
| :--- | :--- |
| `FLOW_MUSIC_MCP_HOME` | Data root directory when `--home` is not set. |
| `FLOW_MUSIC_MCP_CHROME` | Chrome or Chromium binary when `--chrome` is not set. |
| `FLOW_MUSIC_MCP_CHROME_CDP_PORT` | Fixed local CDP port. Default: a free port that Chrome selects. |
| `FLOW_MUSIC_MCP_PAUSE_MIN_S` | Minimum pause between queued jobs in seconds. Default: `5`. |
| `FLOW_MUSIC_MCP_PAUSE_MAX_S` | Maximum pause between queued jobs in seconds. Default: `15`. |

## MCP resources

| URI | Content |
| :--- | :--- |
| `music://downloads/{id}` | Downloaded file by SQLite record ID. |
| `music://downloads/asset/{asset_id}` | Latest downloaded file of a clip or video ID. |

`resources/read` returns files up to 64 MiB. For a larger file, use `music_downloads` with action `read_chunk`.

## Jobs

Queued tools return a job object with an eight-character `id`. Call `music_wait` to get the result. A finished job has `status` `done`, `failed`, or `cancelled`, and these fields:

| Field | Description |
| :--- | :--- |
| `created` | New clip, lyrics, or video IDs. |
| `files` | Local file paths of downloads. |
| `resources` | `music://downloads/{id}` URIs of downloads. |
| `result` | Kind-specific data: clip summaries, lyrics, the Producer reply, or video data. |
| `conversation_id` | Session that the job used. |
| `error` | Error text of a failed job. |

## Session

### `music_status`

Report Google Flow Music session state, job queue, download index, and browser state. Set probe=true to start Chrome and read the account, plan, and credits.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `probe` | boolean | no | Start Chrome if needed and read the account, plan, and credits (default false). |

### `music_login`

Sign in to Google Flow Music through a visible Google Chrome window (action "login"), report the session ("status"), or delete the saved session ("logout").

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `action` | string | no | Action name: "login" (alias "start"), "status", or "logout" (alias "reset_session"). |
| `force` | boolean | no | Delete the saved Chrome profile before interactive sign-in starts (default false). |
| `timeout_secs` | integer | no | Maximum wait time in seconds for sign-in completion (default 180, range 15 to 3600). |
| `wait` | boolean | no | Wait during "login" until sign-in finishes in Chrome (default true). |

## Creation jobs

### `music_generate`

Queue a song generation job with Lyria (spends credits). Give a sound prompt, and optional lyrics, instrumental, BPM, length, title, seed, and prompt image. Lyrics, BPM, and length are hints. The job downloads the audio when it finishes. Use music_wait to get the result.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `bpm` | integer | no | Tempo hint in beats per minute. The value goes into the sound prompt. |
| `download` | boolean | no | Download the finished audio (default true). |
| `format` | string | no | Download format: "m4a" (default), "wav", or "mp3". |
| `image_id` | string | no | Image ID from `music_upload` to use as the prompt image. |
| `instrumental` | boolean | no | Make an instrumental song without vocals (default false). |
| `length_s` | integer | no | Length hint in seconds. The model can make a different length. |
| `lyrics` | string | no | Lyrics text. The model uses the lyrics as guidance and can change them. Omit for model-written lyrics. |
| `lyrics_id` | string | no | Lyrics ID from `music_lyrics`. When set, the tool ignores `lyrics` and `length_s`. |
| `prompt` | string | yes | Sound description: genre, instruments, mood, and vocal style. |
| `seed` | integer | no | Seed for repeatable results. |
| `session_id` | string | no | Existing session ID. Omit to create a new session. |
| `title` | string | no | Song title (default "Untitled"). |

### `music_lyrics`

Queue a lyrics job: write new lyrics from a prompt, or edit existing lyrics. The result gives the lyrics text, a title, and a matching sound prompt.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `action` | string | no | Action: "write" (default) makes new lyrics from `prompt`; "edit" changes `lyrics` or `lyrics_id` with `prompt`. |
| `lyrics` | string | no | Existing lyrics text to edit for "edit". |
| `lyrics_id` | string | no | Existing lyrics ID to edit for "edit". |
| `prompt` | string | yes | Instruction for the lyrics writer. |
| `session_id` | string | no | Existing session ID. Omit to create a new session. |

### `music_edit`

Queue an edit of a song (spends credits): "extend" adds new audio, "replace" rewrites time regions, and "cover" restyles the song but keeps its melody. The result is a new clip.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `action` | string | yes | Edit action: "extend", "replace", or "cover". |
| `clip_id` | string | yes | Source clip ID. |
| `download` | boolean | no | Download the finished audio (default true). |
| `extend_from_s` | number | no | Start point in seconds for "extend". Default: the end of the clip. |
| `extend_s` | number | no | Seconds of new audio for "extend" (default 30). |
| `format` | string | no | Download format: "m4a" (default), "wav", or "mp3". |
| `instruction` | string | no | Instruction for the edit, for example "add a guitar solo". Default: a generic instruction for the action. |
| `regions` | array of Region | no | Regions to rewrite for "replace". |
| `seed` | integer | no | Seed for repeatable results. |
| `session_id` | string | no | Session ID for the edit. Default: the session of the source clip. |
| `strength` | number | no | Similarity to the source for "cover", from 0 to 1 (default 0.5). |
| `title` | string | no | Title of the new clip. Default: the source title followed by the action in parentheses. |

### `music_stems`

Queue a stem separation of a song into vocals, drums, bass, and other. The job downloads the four stems.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `clip_id` | string | yes | Source clip ID. |
| `download` | boolean | no | Download the four stems: vocals, drums, bass, and other (default true). |
| `format` | string | no | Download format: "m4a" (default), "wav", or "mp3". |
| `session_id` | string | no | Session ID for the job. Default: the session of the source clip. |

### `music_chat`

Queue a message to the Google Flow Music Producer agent. The Producer can write lyrics, generate and edit songs, split stems, build Spaces, and plan music videos by itself, thus a message can spend credits. Use a session ID to continue a conversation.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `download` | boolean | no | Download the audio of new clips that the reply creates (default true). |
| `format` | string | no | Download format: "m4a" (default), "wav", or "mp3". |
| `message` | string | yes | Message to the Producer agent. |
| `session_id` | string | no | Existing session ID. Omit to create a new session. |
| `timeout_secs` | integer | no | Maximum wait for the reply in seconds (default 900, range 30 to 3600). |

### `music_video`

List, get, check status of, or delete music videos. Action "create" queues a new video for a song. Kind "social" makes a short vertical 720p clip, waits for the render, and downloads the MP4; the server can make it without credits. Kind "music" makes a custom music video from the song section, the visual direction, and an optional style image; it spends about 12.5 credits per second of video (30 seconds by default), waits for the render (about 5 minutes), and downloads the MP4.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `action` | string | no | Action: "list" (default), "get", "status", "delete", or "create" (queued, spends credits). |
| `aspect_ratio` | string | no | Aspect ratio of a "music" video: "9:16" (default) or "16:9". |
| `clip_id` | string | no | Song clip ID for "create". |
| `download` | boolean | no | Wait for the video to render and download it (default true). |
| `id` | string | no | Music video ID for "get", "status", and "delete". |
| `instructions` | string | no | Visual direction for a "music" video, for example the scene and the style. |
| `kind` | string | no | Video type for "create": "music" (default, a custom music video that spends credits) or "social" (a short vertical clip). |
| `length_s` | integer | no | Video length in seconds (default 15 for "social", 30 for "music"). A "music" video costs about 12.5 credits per second. |
| `limit` | integer | no | Maximum number of videos for "list" (default 20). |
| `offset` | integer | no | Number of videos to skip for "list" (default 0). |
| `session_id` | string | no | Existing session ID for "create". Omit to create a new session. |
| `show_lyrics` | boolean | no | Show the lyrics on screen in a "music" video (default false). |
| `start_s` | number | no | Start point in the song in seconds for a "music" video (default 0). |
| `style_image_url` | string | no | Style reference image URL for a "music" video, for example the image_url from music_upload. |

## Queue control

### `music_wait`

Wait for queued or running Google Flow Music jobs to finish.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `id` | string | no | Single job ID to wait for when `ids` is omitted. |
| `ids` | array of string | no | Job IDs to wait for (waits for all jobs when omitted). |
| `timeout` | integer | no | Maximum wait time in seconds (default 600). One call returns after at most 45 seconds with `pending: true`. |

### `music_cancel`

Cancel a queued or running Google Flow Music job. A queued job spends no credits. For a running job, the server keeps rendering the clips that a tool call already started, and spends their credits; the cancel stops the wait, the download, and the session.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `id` | string | yes | Eight-character job ID from the queue. |

### `music_retry`

Run a failed or cancelled Google Flow Music job again. This spends credits again.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `id` | string | yes | Eight-character job ID from the queue. |

## Library

### `music_songs`

List, get, rename, change privacy of, or delete songs (clips) in the Google Flow Music library.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `action` | string | no | Action: "list" (default), "favorites", "get", "rename", "privacy", or "delete". |
| `ids` | array of string | no | Clip IDs for "get" and "delete", or one clip ID for "rename" and "privacy". |
| `include_disliked` | boolean | no | Include disliked songs for "list" (default false). |
| `limit` | integer | no | Maximum number of songs for "list" and "favorites" (default 20, max 100). |
| `offset` | integer | no | Number of songs to skip for "list" and "favorites" (default 0). |
| `privacy` | string | no | New privacy for "privacy": "public", "unlisted", or "private". |
| `title` | string | no | New title for "rename". |

### `music_sessions`

List, read the transcript of, create, rename, stop, or delete Google Flow Music sessions (Producer conversations).

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `action` | string | no | Action: "list" (default), "get", "create", "rename", "stop", or "delete". |
| `delete_songs` | boolean | no | Also delete the songs of the session for "delete" (default false). |
| `delete_spaces` | boolean | no | Also delete the Spaces of the session for "delete" (default false). |
| `delete_videos` | boolean | no | Also delete the music videos of the session for "delete" (default false). |
| `id` | string | no | Session ID for "get", "rename", "stop", and "delete". |
| `limit` | integer | no | Maximum number of sessions for "list" (default 30, max 100). |
| `offset` | integer | no | Number of sessions to skip for "list" (default 0). |
| `title` | string | no | New title for "rename". |

### `music_playlists`

List, get, create, update, or delete playlists, and add, remove, or reorder songs in a playlist.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `action` | string | no | Action: "list" (default), "favorites", "get", "create", "update", "delete", "add", "remove", or "reorder". |
| `clip_ids` | array of string | no | Clip IDs for "add" and "remove", or the full new order for "reorder". |
| `description` | string | no | Playlist description for "create" and "update". |
| `id` | string | no | Playlist ID for every action except "list", "favorites", and "create". |
| `limit` | integer | no | Maximum number of playlists for "list" (default 20, max 100). |
| `name` | string | no | Playlist name for "create" and "update". |
| `offset` | integer | no | Number of playlists to skip for "list" (default 0). |

### `music_spaces`

List, get, change the privacy of, or delete Spaces (interactive instruments and tools). To create a Space, describe it to the Producer with music_chat.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `action` | string | no | Action: "list" (default), "get", "privacy", or "delete". Create a Space with music_chat. |
| `id` | string | no | Space version ID (the "id" field of "list") for "get", "privacy", and "delete". |
| `include_files` | boolean | no | Include the source files of each Space (default false). |
| `limit` | integer | no | Maximum number of Spaces for "list" (default 20, max 100). |
| `offset` | integer | no | Number of Spaces to skip for "list" (default 0). |
| `privacy` | string | no | New privacy for "privacy": "public", "unlisted", or "private". |

### `music_upload`

Upload a local audio file (becomes a clip that music_edit and music_stems can use) or an image file (gives an image ID for music_generate).

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `kind` | string | no | File kind: "audio" or "image". Default: from the file type. |
| `path` | string | yes | Absolute path of a local audio file (under 4 minutes and 40 MiB) or image file (under 5 MiB). |

## Files

### `music_download`

Download the audio (m4a, wav, or mp3), video, or cover art of a song, or a music video, into the local downloads directory.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `format` | string | no | Audio format for "audio": "m4a" (default), "wav", or "mp3". |
| `id` | string | yes | Clip ID for "audio" and "image", or a clip ID or music video ID for "video". |
| `kind` | string | no | What to download: "audio" (default), "video", or "image" (the cover art). |

### `music_downloads`

List, inspect, read binary chunks, prune, delete, or clear locally downloaded files.

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `action` | string | no | Action name: "list" (default), "get", "read_chunk", "delete", "prune", or "clear". |
| `asset_id` | string | no | Google Flow Music clip or video ID for "get", "read_chunk", or "delete". |
| `existing_only` | boolean | no | Return only rows whose file currently exists on disk for "list" (default false). |
| `id` | integer | no | Download record row ID for "get", "read_chunk", or "delete". |
| `kind` | string | no | File kind filter for "list": "audio", "stem", "video", "image", or "all" (default). |
| `length` | integer | no | Maximum chunk size in bytes for "read_chunk" (1..=4_194_304, default 1_048_576). |
| `limit` | integer | no | Maximum number of download records to return for "list" (1..=500, default 50). |
| `offset` | integer | no | Byte offset in the downloaded file for "read_chunk" (default 0). |
| `older_than_days` | integer | no | Delete only files downloaded more than this many days ago when action is "clear". |
| `query` | string | no | Case-insensitive substring that matches asset ID, name, prompt, file name, or path for "list". |
| `session_id` | string | no | Filter by Google Flow Music session ID for "list". |
