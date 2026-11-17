# 工具参考

本参考列出 `flow-music-mcp` 的命令行选项、环境变量、MCP 资源和 MCP 工具。

## 命令行选项

```text
flow-music-mcp [OPTIONS] <COMMAND>
```

`flow-music-mcp` 没有默认子命令。不带子命令启动时，它会输出帮助并以状态码 `2` 退出。

### 子命令

| 子命令 | 说明 |
| :--- | :--- |
| `serve` | 通过标准输入输出提供 MCP 工具和资源；加上 `--http` 时改用 Streamable HTTP。MCP 客户端使用此子命令。 |
| `login [--timeout <SECS>] [--force] [--json]` | 打开可见的 Chrome 窗口进行 Google 登录，并等待登录完成。`--timeout` 设置最长等待秒数（默认 `180`，范围 `15` 到 `3600`）。`--force` 会先删除已保存的 Chrome 配置文件。 |
| `status [--json]` | 显示登录状态。若存在已保存的配置文件，此子命令会启动无头 Chrome。 |
| `logout [--json]` | 删除 `<home>/chrome-profile`。 |
| `doctor [--json]` | 检查 Chrome 二进制文件、`<home>` 与文件权限、单实例锁以及登录状态。 |

`login`、`status` 和 `logout` 使用与 `music_login` 相同的操作，并为每个结果字段输出一行 `key: value`。加上 `--json` 时，输出 `music_login` 的结果对象。`doctor` 为每项检查输出一行，状态为 `ok`、`warn`、`fail` 或 `skip`。

子命令正常执行完毕时退出状态码为 `0`，即使尚未登录或 `doctor` 有检查未通过也是如此。发生错误时（例如登录超时，或 Chrome 配置文件正被另一个运行中的 `flow-music-mcp` 使用）退出状态码为 `1`。参数值无效时退出状态码为 `2`。

### HTTP 传输

`flow-music-mcp serve --http <IP:PORT>` 会改为在 `http://<IP:PORT>/mcp` 通过 Streamable HTTP 提供 MCP，而不使用标准输入输出。所有 HTTP 客户端共用同一个 Chrome 进程与同一个任务队列。

| `serve` 选项 | 环境变量 | 作用 |
| :--- | :--- | :--- |
| `--http <IP:PORT>` | `FLOW_MUSIC_MCP_HTTP` | 监听此地址，例如 `127.0.0.1:8765` 或 `0.0.0.0:8765` |
| `--http-token <TOKEN>` | `FLOW_MUSIC_MCP_HTTP_TOKEN` | 要求每个请求携带 `Authorization: Bearer <TOKEN>` |

- 未设置令牌时，`flow-music-mcp` 只监听回环地址，并只接受回环地址的 `Host` 请求头。
- 若要接受其他计算机的连接，请设置令牌。设置令牌后，`flow-music-mcp` 可监听任意地址并接受任意 `Host` 请求头。监听非回环地址却未设置令牌时，`flow-music-mcp` 会报错退出。
- 请通过 `FLOW_MUSIC_MCP_HTTP_TOKEN` 设置令牌，而不要写在命令行上，因为同一台计算机的其他用户可以看到命令行。请使用足够长的随机值。
- 服务器使用明文 HTTP。请只在可信网络（例如 VPN）中使用，或在前面加上 TLS 代理。

### 全局选项

这些选项可以写在子命令之前或之后。命令行标志优先于对应的环境变量。

| 选项 | 环境变量 | 说明 |
| :--- | :--- | :--- |
| `--home <PATH>` | `FLOW_MUSIC_MCP_HOME` | 数据根目录。默认值：`~/.flow-music-mcp`。 |
| `--chrome <PATH>` | `FLOW_MUSIC_MCP_CHROME` | Chrome 或 Chromium 二进制文件。默认值：自动检测。 |
| `--cdp-port <PORT>` | `FLOW_MUSIC_MCP_CDP_PORT` | 固定的本地 CDP 端口，范围 `1` 到 `65535`。默认值：由 Chrome 选择的空闲端口。 |
| `--pause <MIN[,MAX]>` | `FLOW_MUSIC_MCP_PAUSE` | 队列中作业之间的随机暂停秒数，范围 `0` 到 `86400`。只写一个值表示固定暂停，`0` 表示不暂停。默认值：`5,15`。 |
| `-h`, `--help` | — | 输出帮助并退出。 |
| `-V`, `--version` | — | 输出版本并退出。 |

参数值无效时，`flow-music-mcp` 会将错误写到标准错误输出，并在启动 Chrome 之前退出。

### 从 0.2 升级

- 在 MCP 客户端配置的 `"args"` 中加入 `serve`。
- 将 `FLOW_MUSIC_MCP_CHROME_CDP_PORT` 改为 `FLOW_MUSIC_MCP_CDP_PORT`。
- 将 `FLOW_MUSIC_MCP_PAUSE_MIN_S` 和 `FLOW_MUSIC_MCP_PAUSE_MAX_S` 改为 `FLOW_MUSIC_MCP_PAUSE`（例如 `5,15`）。

## MCP 资源

| URI | 内容 |
| :--- | :--- |
| `music://downloads/{id}` | 按 SQLite 记录 ID 获取的已下载文件。 |
| `music://downloads/asset/{asset_id}` | 某个片段或视频 ID 最新下载的文件。 |

`resources/read` 返回最大 64 MiB 的文件。对于更大的文件，请使用 `music_downloads` 的 `read_chunk` 操作。

## 作业

排队类工具返回一个带有八个字符 `id` 的作业对象。调用 `music_wait` 获取结果。已结束的作业的 `status` 为 `done`、`failed` 或 `cancelled`，并包含以下字段：

| 字段 | 说明 |
| :--- | :--- |
| `created` | 新的片段、歌词或视频 ID。 |
| `files` | 下载文件的本地路径。 |
| `resources` | 下载文件的 `music://downloads/{id}` URI。 |
| `result` | 与作业类型相关的数据：片段摘要、歌词、Producer 回复或视频数据。 |
| `conversation_id` | 作业使用的会话。 |
| `error` | 失败作业的错误文本。 |

## 会话

### `music_status`

报告 Google Flow Music 会话状态、作业队列、下载索引和浏览器状态。设置 probe=true 可启动 Chrome 并读取账号、套餐和额度。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `probe` | boolean | 否 | 必要时启动 Chrome，并读取账号、套餐和额度（默认 false）。 |

### `music_login`

通过可见的 Google Chrome 窗口登录 Google Flow Music（操作 "login"）、报告会话（"status"）或删除已保存的会话（"logout"）。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `action` | string | 否 | 操作名称："login"（别名 "start"）、"status" 或 "logout"（别名 "reset_session"）。 |
| `force` | boolean | 否 | 在交互式登录开始前删除已保存的 Chrome 配置文件（默认 false）。 |
| `timeout_secs` | integer | 否 | 等待登录完成的最长时间（秒）（默认 180，范围 15 到 3600）。 |
| `wait` | boolean | 否 | 在 "login" 期间等待，直到在 Chrome 中完成登录（默认 true）。 |

## 创作作业

### `music_generate`

将使用 Lyria 的歌曲生成作业加入队列（消耗额度）。提供声音提示词，以及可选的歌词、纯音乐、BPM、时长、标题、种子和提示图片。歌词、BPM 和时长仅作为提示。作业完成后会下载音频。使用 music_wait 获取结果。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `bpm` | integer | 否 | 以每分钟节拍数表示的速度提示。该值会写入声音提示词。 |
| `download` | boolean | 否 | 下载完成的音频（默认 true）。 |
| `format` | string | 否 | 下载格式："m4a"（默认）、"wav" 或 "mp3"。 |
| `image_id` | string | 否 | 来自 `music_upload` 的图片 ID，用作提示图片。 |
| `instrumental` | boolean | 否 | 创作不含人声的纯音乐歌曲（默认 false）。 |
| `length_s` | integer | 否 | 以秒为单位的时长提示。模型可能生成不同的时长。 |
| `lyrics` | string | 否 | 歌词文本。模型将歌词作为参考，并可能修改歌词。省略时由模型撰写歌词。 |
| `lyrics_id` | string | 否 | 来自 `music_lyrics` 的歌词 ID。设置后，该工具会忽略 `lyrics` 和 `length_s`。 |
| `prompt` | string | 是 | 声音描述：曲风、乐器、情绪和演唱风格。 |
| `seed` | integer | 否 | 用于获得可重复结果的种子。 |
| `session_id` | string | 否 | 现有会话 ID。省略时创建新会话。 |
| `title` | string | 否 | 歌曲标题（默认 "Untitled"）。 |

### `music_lyrics`

将歌词作业加入队列：根据提示词撰写新歌词，或编辑现有歌词。结果包含歌词文本、标题和匹配的声音提示词。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `action` | string | 否 | 操作："write"（默认）根据 `prompt` 创作新歌词；"edit" 根据 `prompt` 修改 `lyrics` 或 `lyrics_id`。 |
| `lyrics` | string | 否 | "edit" 要编辑的现有歌词文本。 |
| `lyrics_id` | string | 否 | "edit" 要编辑的现有歌词 ID。 |
| `prompt` | string | 是 | 给歌词撰写者的指令。 |
| `session_id` | string | 否 | 现有会话 ID。省略时创建新会话。 |

### `music_edit`

将歌曲编辑加入队列（消耗额度）："extend" 添加新音频，"replace" 重写时间区域，"cover" 改变歌曲风格但保留旋律。结果是一个新片段。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `action` | string | 是 | 编辑操作："extend"、"replace" 或 "cover"。 |
| `clip_id` | string | 是 | 源片段 ID。 |
| `download` | boolean | 否 | 下载完成的音频（默认 true）。 |
| `extend_from_s` | number | 否 | "extend" 的起始点（秒）。默认值：片段结尾。 |
| `extend_s` | number | 否 | "extend" 新增音频的秒数（默认 30）。 |
| `format` | string | 否 | 下载格式："m4a"（默认）、"wav" 或 "mp3"。 |
| `instruction` | string | 否 | 编辑指令，例如 "add a guitar solo"。默认值：该操作的通用指令。 |
| `regions` | array of Region | 否 | "replace" 要重写的区域。 |
| `seed` | integer | 否 | 用于获得可重复结果的种子。 |
| `session_id` | string | 否 | 编辑使用的会话 ID。默认值：源片段的会话。 |
| `strength` | number | 否 | "cover" 与源歌曲的相似度，范围 0 到 1（默认 0.5）。 |
| `title` | string | 否 | 新片段的标题。默认值：源标题后跟括号中的操作名称。 |

### `music_stems`

将歌曲分轨作业加入队列，分离出人声、鼓、贝斯和其他。作业会下载这四个音轨。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `clip_id` | string | 是 | 源片段 ID。 |
| `download` | boolean | 否 | 下载四个音轨：人声、鼓、贝斯和其他（默认 true）。 |
| `format` | string | 否 | 下载格式："m4a"（默认）、"wav" 或 "mp3"。 |
| `session_id` | string | 否 | 作业使用的会话 ID。默认值：源片段的会话。 |

### `music_chat`

将发送给 Google Flow Music Producer 代理的消息加入队列。Producer 可以自行撰写歌词、生成和编辑歌曲、分离音轨、构建 Spaces 以及策划音乐视频，因此一条消息可能会消耗额度。使用会话 ID 可以继续对话。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `download` | boolean | 否 | 下载回复所创建的新片段的音频（默认 true）。 |
| `format` | string | 否 | 下载格式："m4a"（默认）、"wav" 或 "mp3"。 |
| `message` | string | 是 | 发送给 Producer 代理的消息。 |
| `session_id` | string | 否 | 现有会话 ID。省略时创建新会话。 |
| `timeout_secs` | integer | 否 | 等待回复的最长时间（秒）（默认 900，范围 30 到 3600）。 |

### `music_video`

列出、获取、查看状态或删除音乐视频。操作 "create" 会为歌曲排队创建新视频。类型 "social" 制作简短的 720p 竖屏片段，等待渲染完成并下载 MP4；服务器可能不收取额度。类型 "music" 根据歌曲片段、视觉指导和可选的风格图片制作自定义音乐视频；每秒视频约消耗 12.5 额度（默认 30 秒），等待渲染完成（约 5 分钟）并下载 MP4。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `action` | string | 否 | 操作："list"（默认）、"get"、"status"、"delete" 或 "create"（排队，消耗额度）。 |
| `aspect_ratio` | string | 否 | "music" 视频的宽高比："9:16"（默认）或 "16:9"。 |
| `clip_id` | string | 否 | "create" 使用的歌曲片段 ID。 |
| `download` | boolean | 否 | 等待视频渲染完成并下载（默认 true）。 |
| `id` | string | 否 | "get"、"status" 和 "delete" 使用的音乐视频 ID。 |
| `instructions` | string | 否 | "music" 视频的视觉指导，例如场景和风格。 |
| `kind` | string | 否 | "create" 的视频类型："music"（默认，消耗额度的自定义音乐视频）或 "social"（简短的竖屏片段）。 |
| `length_s` | integer | 否 | 视频时长（秒）（"social" 默认 15，"music" 默认 30）。"music" 视频每秒约消耗 12.5 额度。 |
| `limit` | integer | 否 | "list" 返回的最大视频数（默认 20）。 |
| `offset` | integer | 否 | "list" 跳过的视频数（默认 0）。 |
| `session_id` | string | 否 | "create" 使用的现有会话 ID。省略时创建新会话。 |
| `show_lyrics` | boolean | 否 | 在 "music" 视频中显示歌词（默认 false）。 |
| `start_s` | number | 否 | "music" 视频在歌曲中的起始时间（秒）（默认 0）。 |
| `style_image_url` | string | 否 | "music" 视频的风格参考图片 URL，例如 music_upload 返回的 image_url。 |

## 队列控制

### `music_wait`

等待排队中或运行中的 Google Flow Music 作业完成。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `id` | string | 否 | 省略 `ids` 时要等待的单个作业 ID。 |
| `ids` | array of string | 否 | 要等待的作业 ID（省略时等待所有作业）。 |
| `timeout` | integer | 否 | 最长等待时间（秒）（默认 600）。每次调用最多 45 秒后返回 `pending: true`。 |

### `music_cancel`

取消排队中或运行中的 Google Flow Music 作业。排队中的作业不消耗 credits。对于运行中的作业，服务器会继续渲染工具调用已开始的 clip，并扣除相应 credits；取消只会停止等待、下载和会话。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `id` | string | 是 | 队列中八个字符的作业 ID。 |

### `music_retry`

重新运行失败或已取消的 Google Flow Music 作业。这会再次消耗额度。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `id` | string | 是 | 队列中八个字符的作业 ID。 |

## 资料库

### `music_songs`

列出、获取、重命名、更改隐私设置或删除 Google Flow Music 资料库中的歌曲（片段）。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `action` | string | 否 | 操作："list"（默认）、"favorites"、"get"、"rename"、"privacy" 或 "delete"。 |
| `ids` | array of string | 否 | "get" 和 "delete" 使用的片段 ID，或 "rename" 和 "privacy" 使用的单个片段 ID。 |
| `include_disliked` | boolean | 否 | "list" 时包含标记为不喜欢的歌曲（默认 false）。 |
| `limit` | integer | 否 | "list" 和 "favorites" 返回的最大歌曲数（默认 20，最大 100）。 |
| `offset` | integer | 否 | "list" 和 "favorites" 跳过的歌曲数（默认 0）。 |
| `privacy` | string | 否 | "privacy" 的新隐私设置："public"、"unlisted" 或 "private"。 |
| `title` | string | 否 | "rename" 的新标题。 |

### `music_sessions`

列出、读取记录、创建、重命名、停止或删除 Google Flow Music 会话（Producer 对话）。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `action` | string | 否 | 操作："list"（默认）、"get"、"create"、"rename"、"stop" 或 "delete"。 |
| `delete_songs` | boolean | 否 | "delete" 时同时删除该会话的歌曲（默认 false）。 |
| `delete_spaces` | boolean | 否 | "delete" 时同时删除该会话的 Spaces（默认 false）。 |
| `delete_videos` | boolean | 否 | "delete" 时同时删除该会话的音乐视频（默认 false）。 |
| `id` | string | 否 | "get"、"rename"、"stop" 和 "delete" 使用的会话 ID。 |
| `limit` | integer | 否 | "list" 返回的最大会话数（默认 30，最大 100）。 |
| `offset` | integer | 否 | "list" 跳过的会话数（默认 0）。 |
| `title` | string | 否 | "rename" 的新标题。 |

### `music_playlists`

列出、获取、创建、更新或删除播放列表，以及在播放列表中添加、移除或重新排序歌曲。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `action` | string | 否 | 操作："list"（默认）、"favorites"、"get"、"create"、"update"、"delete"、"add"、"remove" 或 "reorder"。 |
| `clip_ids` | array of string | 否 | "add" 和 "remove" 使用的片段 ID，或 "reorder" 的完整新顺序。 |
| `description` | string | 否 | "create" 和 "update" 使用的播放列表描述。 |
| `id` | string | 否 | 除 "list"、"favorites" 和 "create" 外所有操作使用的播放列表 ID。 |
| `limit` | integer | 否 | "list" 返回的最大播放列表数（默认 20，最大 100）。 |
| `name` | string | 否 | "create" 和 "update" 使用的播放列表名称。 |
| `offset` | integer | 否 | "list" 跳过的播放列表数（默认 0）。 |

### `music_spaces`

列出、获取、更改隐私设置或删除 Spaces（交互式乐器和工具）。如需创建 Space，请通过 music_chat 向 Producer 描述它。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `action` | string | 否 | 操作："list"（默认）、"get"、"privacy" 或 "delete"。使用 music_chat 创建 Space。 |
| `id` | string | 否 | "get"、"privacy" 和 "delete" 使用的 Space 版本 ID（"list" 返回的 "id" 字段）。 |
| `include_files` | boolean | 否 | 包含每个 Space 的源文件（默认 false）。 |
| `limit` | integer | 否 | "list" 返回的最大 Space 数（默认 20，最大 100）。 |
| `offset` | integer | 否 | "list" 跳过的 Space 数（默认 0）。 |
| `privacy` | string | 否 | "privacy" 的新隐私设置："public"、"unlisted" 或 "private"。 |

### `music_upload`

上传本地音频文件（成为 music_edit 和 music_stems 可以使用的片段）或图片文件（提供供 music_generate 使用的图片 ID）。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `kind` | string | 否 | 文件类型："audio" 或 "image"。默认值：根据文件类型判断。 |
| `path` | string | 是 | 本地音频文件（不超过 4 分钟且小于 40 MiB）或图片文件（小于 5 MiB）的绝对路径。 |

## 文件

### `music_download`

将歌曲的音频（m4a、wav 或 mp3）、视频或封面图片，或音乐视频下载到本地下载目录。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `format` | string | 否 | "audio" 的音频格式："m4a"（默认）、"wav" 或 "mp3"。 |
| `id` | string | 是 | "audio" 和 "image" 使用的片段 ID，或 "video" 使用的片段 ID 或音乐视频 ID。 |
| `kind` | string | 否 | 下载内容："audio"（默认）、"video" 或 "image"（封面图片）。 |

### `music_downloads`

列出、查看、读取二进制分块、清理、删除或清空本地已下载的文件。

| 参数 | 类型 | 必需 | 说明 |
| :--- | :--- | :--- | :--- |
| `action` | string | 否 | 操作名称："list"（默认）、"get"、"read_chunk"、"delete"、"prune" 或 "clear"。 |
| `asset_id` | string | 否 | "get"、"read_chunk" 或 "delete" 使用的 Google Flow Music 片段或视频 ID。 |
| `existing_only` | boolean | 否 | "list" 时只返回文件当前存在于磁盘上的记录（默认 false）。 |
| `id` | integer | 否 | "get"、"read_chunk" 或 "delete" 使用的下载记录行 ID。 |
| `kind` | string | 否 | "list" 的文件类型筛选："audio"、"stem"、"video"、"image" 或 "all"（默认）。 |
| `length` | integer | 否 | "read_chunk" 的最大分块大小（字节）（1..=4_194_304，默认 1_048_576）。 |
| `limit` | integer | 否 | "list" 返回的最大下载记录数（1..=500，默认 50）。 |
| `offset` | integer | 否 | "read_chunk" 在已下载文件中的字节偏移量（默认 0）。 |
| `older_than_days` | integer | 否 | 操作为 "clear" 时，只删除下载时间早于此天数的文件。 |
| `query` | string | 否 | "list" 使用的不区分大小写的子字符串，匹配资源 ID、名称、提示词、文件名或路径。 |
| `session_id` | string | 否 | "list" 时按 Google Flow Music 会话 ID 筛选。 |
