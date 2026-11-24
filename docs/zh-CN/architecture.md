# 架构

本指南说明 `flow-music-mcp` 的工作原理。

## 组件

| 组件 | 源码 | 职责 |
| :--- | :--- | :--- |
| MCP 适配器 | `src/mcp.rs` | 通过 `rmcp` 注册 18 个工具和 `music://downloads/...` 资源。耗时较长的工具在 45 秒后返回 `pending: true`。 |
| 服务 | `src/service.rs` | 路由工具调用、将作业加入队列，并提供下载文件和资源。 |
| `ChromeEngine` | `src/engine.rs`、`src/chrome.rs`、`src/cdp.rs` | 启动 Chrome，在应用源上保持一个页面，并运行页面运行时。 |
| 页面运行时 | `ts/*.ts` | 读取页面的会话，并发送 API 请求、流读取、上传和二进制下载。 |
| API 客户端 | `src/client.rs` | 调用页面运行时，并在会话过期时通过重新加载页面刷新一次会话。 |
| Producer 层 | `src/producer.rs` | 发送 Producer 工具调用和对话消息，跟踪作业流，并等待片段渲染完成。 |
| 领域模块 | `src/generation.rs`、`src/edits.rs`、`src/chat.rs`、`src/library.rs`、`src/collections.rs`、`src/uploads.rs` | 为每个工具构建请求。 |
| `JobQueue` | `src/queue.rs` | 逐个运行消耗额度的作业。 |
| 存储 | `src/db.rs`、`src/download.rs`、`src/media_files.rs` | 将文件保存到 `<home>/downloads`，并在 SQLite 中建立索引。 |

## 登录与会话

Google Flow Music 将会话保存在 `www.flowmusic.app` 上的 Supabase Cookie（`sb-sb-auth-token.0`、`sb-sb-auth-token.1` 以及更多分块）中。会话只存在于 `<home>/chrome-profile` 的持久化 Chrome 配置文件中。服务器不会将会话 Cookie 复制到其他文件。

1. `music_login` 在 `<home>/chrome-profile` 上打开一个不带 CDP 参数的可见 Chrome 窗口。
2. 服务器每秒读取一次 Chrome `Cookies` 和 `History` 数据库的临时副本。它查找会话 Cookie 和 Google 账号的拒绝页面。
3. 当会话 Cookie 存在，或用户关闭窗口时，服务器停止可见的 Chrome，并在同一配置文件上启动无头 Chrome。
4. 页面运行时通过 `GET /__api/users/me` 确认会话。

访问令牌大约在一小时后过期。当运行时发现令牌过期，或 API 返回 HTTP 401 时，客户端会重新加载页面。随后页面客户端刷新令牌，客户端再重新发送一次请求。

### 单实例锁与孤儿 Chrome

- 启动 Chrome 前，`flow-music-mcp` 会对 `<home>/flow-music-mcp.lock` 加独占锁，并持有到进程结束。进程结束时（包括崩溃），操作系统会释放该锁。
- 若锁已被其他进程持有，启动 Chrome、`login`、`status`、`logout` 以及 `music_login` 的 `logout` 操作都会报错，`doctor` 会显示警告。
- 持有锁期间，`<home>/chrome-profile` 上任何不是由本进程启动的 Chrome 进程都视为孤儿进程。`flow-music-mcp` 会先请求其正常退出（Unix 上发送 `SIGINT`，Windows 上执行 `taskkill`），3 秒后仍未退出则强制结束。

## 来自页面的请求

内部 API 位于 `https://www.flowmusic.app/__api/...`。每个请求都携带 `Authorization: Bearer <access token>`。运行时从页面使用 `fetch` 发送所有请求，因此这些请求与网站具有相同的源、Cookie 和浏览器身份。

Rust 端只使用固定的 API 路径。任何工具都不能向调用方指定的路径发送请求。

## Producer 工具调用

歌曲创作、编辑、分轨、歌词和视频都属于 Producer 工具：

1. `POST /__api/producer/tool-call` 发送一个工具调用并返回 `job_id`。
2. `GET /__api/messages/<job_id>/stream` 发送服务器推送事件 (SSE)。运行时以 20 秒为一段读取该流，并使用 `last_id` 继续读取。
3. 具有相同 `tool_call_id` 的 `tool-return` 部分携带结果，例如新的片段 ID。
4. 工具返回后，片段可能仍在渲染。服务器轮询 `POST /__api/clips`，直到每个片段的音频完成。

编辑使用配方 (recipe)。`POST /__api/recipes/create` 存储三个节点：输入片段、一个 `ExtendSection`、`ReplaceSection` 或 `CoverSong` 节点，以及输出。然后 `audio__render_edit` 工具渲染该配方。

`music_generate` 构建的参数与网站的创作表单相同。BPM 写入声音提示词，时长以 `[End - m:ss]` 标记写入歌词。歌词 ID 是歌词文本的 UUID 第 5 版，歌词文本放在客户端上下文中。模型将歌词、BPM 和时长作为参考。

`music_chat` 发送 `POST /__api/conversation`。Producer 代理会自行选择工具，因此一条对话消息可能会消耗额度。

## 作业队列

`JobQueue` 在一个工作任务上逐个运行作业。在两个作业之间，工作任务会等待 `--pause` 或 `FLOW_MUSIC_MCP_PAUSE` 指定范围内的随机时间（默认 5 到 15 秒）。

- 队列从不自动重试作业。`music_retry` 会重新运行失败或已取消的作业，并再次消耗额度。
- `music_cancel` 停止本地作业。如果该作业有会话，服务器还会发送 `POST /__api/conversations/<id>/stop`。工具调用已开始的 clip 仍会在服务器上继续渲染并消耗 credits。
- 队列在内存中保留最近 200 个作业。服务器重启会清空队列，但歌曲仍保留在您的资料库中。

## 存储

所有数据都位于 `<home>`（默认 `~/.flow-music-mcp`）下：

| 路径 | 内容 | Unix 权限 |
| :--- | :--- | :--- |
| `chrome-profile/` | 包含会话的持久化 Chrome 配置文件 | `0700` |
| `flow-music-mcp.lock` | 进程可启动 Chrome 期间持有的单实例锁 | `0600` |
| `flow-music-mcp.db` | 下载文件的 SQLite 索引 | `0600` |
| `downloads/` | 下载的音频、视频和图片 | `0700` 目录 |

M4A、WAV、分轨和封面文件来自公开存储 URL。MP3 文件通过页面从 `GET /__api/download/audio/<clip_id>?format=mp3` 获取。服务器在为每个文件建立索引之前会检查文件的开头字节。
